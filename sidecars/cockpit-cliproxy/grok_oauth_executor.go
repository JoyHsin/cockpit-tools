package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"sort"
	"strings"

	"github.com/gin-gonic/gin"
	responsesconverter "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/openai/openai/responses"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

const grokCLIChatURL = "https://cli-chat-proxy.grok.com/v1/chat/completions"

type grokPoolState struct {
	affinity *unifiedSessionAffinity
}

func newGrokPoolState() *grokPoolState {
	return &grokPoolState{affinity: newUnifiedSessionAffinity()}
}

func grokMembersForRoute(cfg *unifiedGatewayConfig, model string) []unifiedGrokPoolMember {
	if cfg == nil {
		return nil
	}
	members := append([]unifiedGrokPoolMember(nil), cfg.GrokPool...)
	filtered := members[:0]
	for _, member := range members {
		if strings.TrimSpace(member.AccountID) == "" {
			continue
		}
		if len(member.AllowedModels) > 0 && !containsFold(member.AllowedModels, model) {
			continue
		}
		filtered = append(filtered, member)
	}
	sort.SliceStable(filtered, func(i, j int) bool {
		if filtered[i].BackupOnly != filtered[j].BackupOnly {
			return !filtered[i].BackupOnly
		}
		if filtered[i].Priority != filtered[j].Priority {
			return filtered[i].Priority < filtered[j].Priority
		}
		return filtered[i].Weight > filtered[j].Weight
	})
	return filtered
}

func containsFold(values []string, wanted string) bool {
	for _, value := range values {
		if strings.EqualFold(strings.TrimSpace(value), wanted) {
			return true
		}
	}
	return false
}

func (s *relayServer) handleGrokOAuthRequest(c *gin.Context, body []byte, route *unifiedGatewayRoute, sourceFormat sdktranslator.Format, fixedAlt string) {
	if s.grokPool == nil {
		s.grokPool = newGrokPoolState()
	}
	cfg := s.manifest.UnifiedGateway
	members := grokMembersForRoute(cfg, route.ModelID)
	if len(members) == 0 {
		writeAPIError(c, http.StatusConflict, "Grok 需要重新授权", "grok_reauth_required")
		return
	}
	sessionID := requestSessionID(body, c.Request.Header)
	ordered := orderGrokMembers(members, s.grokPool.affinity.get(sessionID))
	stream := requestBodyStream(body) && fixedAlt != "responses/compact"
	wrote := false
	var lastErr error
	for index, member := range ordered {
		if wrote {
			break
		}
		err := s.executeGrokMember(c, body, route, member, sourceFormat, stream, &wrote)
		if err == nil {
			s.grokPool.affinity.set(sessionID, member.AccountID)
			return
		}
		lastErr = err
		if !canRetryGrokError(err, wrote, index, len(ordered)) {
			break
		}
	}
	if lastErr == nil {
		lastErr = fmt.Errorf("Grok 需要重新授权")
	}
	if wrote {
		writeSSEError(c, lastErr.Error())
		return
	}
	status, code := classifyGrokExecutorError(lastErr)
	writeAPIError(c, status, lastErr.Error(), code)
}

func orderGrokMembers(members []unifiedGrokPoolMember, affinity string) []unifiedGrokPoolMember {
	if affinity == "" {
		return members
	}
	ordered := make([]unifiedGrokPoolMember, 0, len(members))
	var rest []unifiedGrokPoolMember
	for _, member := range members {
		if member.AccountID == affinity {
			ordered = append(ordered, member)
		} else {
			rest = append(rest, member)
		}
	}
	return append(ordered, rest...)
}

func canRetryGrokError(err error, wrote bool, index, total int) bool {
	if wrote || err == nil || index+1 >= total {
		return false
	}
	message := strings.ToLower(err.Error())
	return strings.Contains(message, "401") ||
		strings.Contains(message, "invalid_grant") ||
		strings.Contains(message, "429") ||
		strings.Contains(message, "reauth") ||
		strings.Contains(message, "quota") ||
		strings.Contains(message, "connect")
}

func classifyGrokExecutorError(err error) (int, string) {
	message := strings.ToLower(err.Error())
	switch {
	case strings.Contains(message, "reauth") || strings.Contains(message, "invalid_grant"):
		return http.StatusUnauthorized, "grok_reauth_required"
	case strings.Contains(message, "429") || strings.Contains(message, "quota"):
		return http.StatusTooManyRequests, "grok_quota_exhausted"
	case strings.Contains(message, "model"):
		return http.StatusBadRequest, "grok_model_unavailable"
	default:
		return http.StatusBadGateway, "grok_upstream_failed"
	}
}

func (s *relayServer) executeGrokMember(c *gin.Context, body []byte, route *unifiedGatewayRoute, member unifiedGrokPoolMember, sourceFormat sdktranslator.Format, stream bool, wrote *bool) error {
	broker := getGlobalBroker()
	if broker == nil {
		return fmt.Errorf("credential broker is not connected")
	}
	token, err := broker.GetGrokAccessToken(member.AccountID)
	if err != nil {
		if strings.Contains(strings.ToLower(err.Error()), "reauth") || strings.Contains(strings.ToLower(err.Error()), "invalid_grant") {
			broker.MarkGrokAccount(member.AccountID, "reauth_required")
		}
		return err
	}
	upstreamModel := route.UpstreamModel
	if upstreamModel == "" {
		upstreamModel = route.ModelID
	}
	chatBody := convertResponsesToChat(body, upstreamModel, sourceFormat)
	req, err := http.NewRequestWithContext(contextOrBackground(c), http.MethodPost, grokCLIChatURL, bytes.NewReader(chatBody))
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "text/event-stream")
	req.Header.Set("x-xai-token-auth", "xai-grok-cli")
	client := &http.Client{Timeout: 0}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("grok connect failed: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized {
		broker.MarkGrokAccount(member.AccountID, "reauth_required")
		return fmt.Errorf("401 unauthorized")
	}
	if resp.StatusCode == http.StatusTooManyRequests {
		return fmt.Errorf("429 quota")
	}
	if resp.StatusCode >= 500 {
		return fmt.Errorf("grok upstream %d", resp.StatusCode)
	}
	if resp.StatusCode >= 400 {
		payload, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
		return fmt.Errorf("grok upstream %d: %s", resp.StatusCode, strings.TrimSpace(string(payload)))
	}
	if stream {
		return writeGrokChatAsResponsesSSE(c, resp.Body, wrote)
	}
	payload, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	*wrote = true
	c.Data(http.StatusOK, "application/json", convertChatToResponses(payload))
	return nil
}

func convertResponsesToChat(body []byte, model string, sourceFormat sdktranslator.Format) []byte {
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI) {
		return rewriteRequestModel(body, model)
	}
	converted := responsesconverter.ConvertOpenAIResponsesRequestToOpenAIChatCompletions(model, rewriteRequestModel(body, model), true)
	if len(bytes.TrimSpace(converted)) == 0 {
		return fallbackResponsesToChat(body, model)
	}
	return converted
}

func fallbackResponsesToChat(body []byte, model string) []byte {
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return []byte(fmt.Sprintf(`{"model":%q,"stream":true,"messages":[{"role":"user","content":""}]}`, model))
	}
	messages := extractChatMessages(payload["input"])
	if len(messages) == 0 {
		messages = []map[string]any{{"role": "user", "content": ""}}
	}
	out := map[string]any{
		"model":    model,
		"stream":   true,
		"messages": messages,
	}
	if tools, ok := payload["tools"]; ok {
		out["tools"] = tools
	}
	raw, _ := json.Marshal(out)
	return raw
}

func extractChatMessages(input any) []map[string]any {
	switch typed := input.(type) {
	case string:
		if strings.TrimSpace(typed) == "" {
			return nil
		}
		return []map[string]any{{"role": "user", "content": typed}}
	case []any:
		var messages []map[string]any
		for _, item := range typed {
			object, ok := item.(map[string]any)
			if !ok {
				continue
			}
			role, _ := object["role"].(string)
			if role == "" {
				role = "user"
			}
			if content, ok := object["content"].(string); ok {
				messages = append(messages, map[string]any{"role": role, "content": content})
				continue
			}
			if parts, ok := object["content"].([]any); ok {
				var text strings.Builder
				for _, part := range parts {
					partObject, ok := part.(map[string]any)
					if !ok {
						continue
					}
					if value, ok := partObject["text"].(string); ok {
						text.WriteString(value)
					}
				}
				messages = append(messages, map[string]any{"role": role, "content": text.String()})
			}
		}
		return messages
	default:
		return nil
	}
}

func convertChatToResponses(payload []byte) []byte {
	converted := responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponsesNonStream(contextOrBackground(nil), "", nil, nil, payload, nil)
	if len(bytes.TrimSpace(converted)) > 0 {
		return converted
	}
	return payload
}

func writeGrokChatAsResponsesSSE(c *gin.Context, body io.Reader, wrote *bool) error {
	c.Header("Content-Type", "text/event-stream")
	c.Header("Cache-Control", "no-cache")
	c.Status(http.StatusOK)
	*wrote = true
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 2*1024*1024)
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(bytes.TrimSpace(line)) == 0 {
			continue
		}
		data := line
		if bytes.HasPrefix(data, []byte("data:")) {
			data = bytes.TrimSpace(bytes.TrimPrefix(data, []byte("data:")))
		}
		if bytes.Equal(data, []byte("[DONE]")) {
			_, _ = io.WriteString(c.Writer, "event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n")
			break
		}
		converted := convertChatToResponses(data)
		if _, err := fmt.Fprintf(c.Writer, "event: response.output_text.delta\ndata: %s\n\n", converted); err != nil {
			return err
		}
		if flusher, ok := c.Writer.(http.Flusher); ok {
			flusher.Flush()
		}
	}
	return scanner.Err()
}

func (s *relayServer) handleBrokerProviderRequest(c *gin.Context, body []byte, route *unifiedGatewayRoute) {
	broker := getGlobalBroker()
	if broker == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "credential broker is not connected", "broker_unavailable")
		return
	}
	resp, err := broker.ExecuteProvider(route.ProviderID, route.UpstreamModel, json.RawMessage(rewriteRequestModel(body, route.UpstreamModel)))
	if err != nil {
		writeAPIError(c, http.StatusBadGateway, err.Error(), "provider_execute_failed")
		return
	}
	status := http.StatusOK
	if raw, ok := resp["status"].(float64); ok && raw > 0 {
		status = int(raw)
	}
	contentType, _ := resp["contentType"].(string)
	if contentType == "" {
		contentType = "application/json"
	}
	payload, _ := resp["body"].(string)
	c.Data(status, contentType, []byte(payload))
}
