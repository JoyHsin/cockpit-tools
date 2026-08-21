package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"sort"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	responsesconverter "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/openai/openai/responses"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

const grokCLIChatURL = "https://cli-chat-proxy.grok.com/v1/chat/completions"

const grokCLIVersion = "0.1.205"
const compactSummaryPrefix = "cockpit-compact-v1:"

func applyGrokCLIHeaders(req *http.Request, token string, stream bool) {
	if req == nil {
		return
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("Content-Type", "application/json")
	if stream {
		req.Header.Set("Accept", "text/event-stream")
	} else {
		req.Header.Set("Accept", "application/json")
	}
	req.Header.Set("User-Agent", "grok-cli/"+grokCLIVersion)
	req.Header.Set("x-xai-token-auth", "xai-grok-cli")
	req.Header.Set("x-grok-cli-version", grokCLIVersion)
	req.Header.Set("x-grok-client-version", grokCLIVersion)
	req.Header.Set("x-grok-client-surface", "grok-cli")
	req.Header.Set("x-grok-client-identifier", "cockpit-tools")
}

type grokPoolState struct {
	affinity *unifiedSessionAffinity
}

func newGrokPoolState() *grokPoolState {
	return &grokPoolState{affinity: newUnifiedSessionAffinity()}
}

func grokMembersForRoute(cfg *unifiedGatewayConfig, model, upstreamModel string) []unifiedGrokPoolMember {
	if cfg == nil {
		return nil
	}
	members := append([]unifiedGrokPoolMember(nil), cfg.GrokPool...)
	filtered := members[:0]
	for _, member := range members {
		if strings.TrimSpace(member.AccountID) == "" {
			continue
		}
		if len(member.AllowedModels) > 0 &&
			!containsFold(member.AllowedModels, model) &&
			!containsFold(member.AllowedModels, upstreamModel) {
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
	members := grokMembersForRoute(cfg, route.ModelID, route.UpstreamModel)
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
		err := s.executeGrokMember(c, body, route, member, sourceFormat, fixedAlt, stream, &wrote)
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

func (s *relayServer) executeGrokMember(c *gin.Context, body []byte, route *unifiedGatewayRoute, member unifiedGrokPoolMember, sourceFormat sdktranslator.Format, fixedAlt string, stream bool, wrote *bool) error {
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
	compact := fixedAlt == "responses/compact"
	requestBody := body
	if compact {
		requestBody = buildCompactionRequest(body)
	}
	chatBody := convertResponsesToChat(requestBody, upstreamModel, sourceFormat, stream)
	req, err := http.NewRequestWithContext(contextOrBackground(c), http.MethodPost, grokCLIChatURL, bytes.NewReader(chatBody))
	if err != nil {
		return err
	}
	applyGrokCLIHeaders(req, token, stream)
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
		return writeGrokChatAsResponsesSSE(c, resp.Body, body, chatBody, upstreamModel, wrote)
	}
	payload, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	*wrote = true
	if compact {
		summary := extractChatResponseText(payload)
		if strings.TrimSpace(summary) == "" {
			return fmt.Errorf("grok compact response did not contain a summary")
		}
		writeCompactionResponse(c, route.ModelID, summary)
		return nil
	}
	c.Data(http.StatusOK, "application/json", convertChatToResponses(payload, upstreamModel, body, chatBody))
	return nil
}

func websocketMessageToResponsesBody(raw []byte) []byte {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil || payload == nil {
		return raw
	}
	delete(payload, "type")
	payload["stream"] = true
	out, err := json.Marshal(payload)
	if err != nil {
		return raw
	}
	return out
}

func isWebsocketPrewarm(raw []byte) bool {
	var env struct {
		Type     string `json:"type"`
		Generate *bool  `json:"generate"`
	}
	if json.Unmarshal(raw, &env) != nil {
		return false
	}
	return env.Type == "response.create" && env.Generate != nil && !*env.Generate
}

func writeWebsocketPrewarm(conn *websocket.Conn, raw []byte) error {
	model := websocketEnvelopeModel(raw)
	now := time.Now().Unix()
	created := gin.H{
		"type":            "response.created",
		"sequence_number": 0,
		"response": gin.H{
			"id":         "resp_prewarm_grok",
			"object":     "response",
			"created_at": now,
			"status":     "in_progress",
			"background": false,
			"error":      nil,
			"output":     []any{},
			"model":      model,
		},
	}
	completed := gin.H{
		"type":            "response.completed",
		"sequence_number": 1,
		"response": gin.H{
			"id":         "resp_prewarm_grok",
			"object":     "response",
			"created_at": now,
			"status":     "completed",
			"background": false,
			"error":      nil,
			"output":     []any{},
			"model":      model,
			"usage":      gin.H{"input_tokens": 0, "output_tokens": 0, "total_tokens": 0},
		},
	}
	if err := conn.WriteJSON(created); err != nil {
		return err
	}
	return conn.WriteJSON(completed)
}

func (s *relayServer) executeGrokWebsocketTurn(conn *websocket.Conn, raw []byte, route *unifiedGatewayRoute) error {
	if isWebsocketPrewarm(raw) {
		return writeWebsocketPrewarm(conn, raw)
	}
	if s.grokPool == nil {
		s.grokPool = newGrokPoolState()
	}
	cfg := s.manifest.UnifiedGateway
	body := websocketMessageToResponsesBody(raw)
	members := grokMembersForRoute(cfg, route.ModelID, route.UpstreamModel)
	if len(members) == 0 {
		return fmt.Errorf("Grok 需要重新授权")
	}
	sessionID := requestSessionID(body, nil)
	ordered := orderGrokMembers(members, s.grokPool.affinity.get(sessionID))
	var lastErr error
	for index, member := range ordered {
		err := s.executeGrokMemberWebsocket(conn, body, route, member)
		if err == nil {
			s.grokPool.affinity.set(sessionID, member.AccountID)
			return nil
		}
		lastErr = err
		if !canRetryGrokError(err, false, index, len(ordered)) {
			break
		}
	}
	if lastErr == nil {
		lastErr = fmt.Errorf("Grok 需要重新授权")
	}
	return lastErr
}

func (s *relayServer) executeGrokMemberWebsocket(conn *websocket.Conn, body []byte, route *unifiedGatewayRoute, member unifiedGrokPoolMember) error {
	broker := getGlobalBroker()
	if broker == nil {
		return fmt.Errorf("credential broker is not connected")
	}
	token, err := broker.GetGrokAccessToken(member.AccountID)
	if err != nil {
		return err
	}
	upstreamModel := route.UpstreamModel
	if upstreamModel == "" {
		upstreamModel = route.ModelID
	}
	chatBody := convertResponsesToChat(body, upstreamModel, sdktranslator.FormatOpenAIResponse, true)
	req, err := http.NewRequest(http.MethodPost, grokCLIChatURL, bytes.NewReader(chatBody))
	if err != nil {
		return err
	}
	applyGrokCLIHeaders(req, token, true)
	client := &http.Client{Timeout: 120 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("grok connect failed: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized {
		broker.MarkGrokAccount(member.AccountID, "reauth_required")
		return fmt.Errorf("401 unauthorized")
	}
	if resp.StatusCode >= 400 {
		payload, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
		return fmt.Errorf("grok upstream %d: %s", resp.StatusCode, strings.TrimSpace(string(payload)))
	}
	return writeGrokChatAsResponsesWebsocket(conn, resp.Body, body, upstreamModel)
}

func writeGrokChatAsResponsesWebsocket(conn *websocket.Conn, body io.Reader, requestJSON []byte, model string) error {
	var param any
	ctx := context.Background()
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 2*1024*1024)
	wrote := false
	for scanner.Scan() {
		line := bytes.TrimSpace(scanner.Bytes())
		if len(line) == 0 {
			continue
		}
		events := responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponses(
			ctx, model, requestJSON, requestJSON, line, &param,
		)
		for _, event := range events {
			if err := conn.WriteMessage(websocket.TextMessage, event); err != nil {
				return err
			}
			wrote = true
		}
	}
	if err := scanner.Err(); err != nil {
		return err
	}
	flush := responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponses(
		ctx, model, requestJSON, requestJSON, []byte("[DONE]"), &param,
	)
	for _, event := range flush {
		if err := conn.WriteMessage(websocket.TextMessage, event); err != nil {
			return err
		}
		wrote = true
	}
	if !wrote {
		return fmt.Errorf("grok returned an empty stream")
	}
	return nil
}

func convertResponsesToChat(body []byte, model string, sourceFormat sdktranslator.Format, stream bool) []byte {
	body = expandCompactionItems(body)
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI) {
		out := rewriteRequestModel(body, model)
		return rewriteRequestStream(out, stream)
	}
	converted := responsesconverter.ConvertOpenAIResponsesRequestToOpenAIChatCompletions(model, rewriteRequestModel(body, model), stream)
	if len(bytes.TrimSpace(converted)) == 0 {
		return fallbackResponsesToChatWithStream(body, model, stream)
	}
	return converted
}

func convertChatToResponsesRequest(body []byte, model string, stream bool) []byte {
	var payload map[string]any
	if json.Unmarshal(body, &payload) != nil || payload == nil {
		return body
	}
	messages, _ := payload["messages"].([]any)
	input := make([]any, 0, len(messages))
	for _, raw := range messages {
		message, ok := raw.(map[string]any)
		if !ok {
			continue
		}
		role, _ := message["role"].(string)
		if role == "tool" {
			callID, _ := message["tool_call_id"].(string)
			input = append(input, map[string]any{
				"type":    "function_call_output",
				"call_id": callID,
				"output":  chatContentText(message["content"]),
			})
			continue
		}
		if toolCalls, ok := message["tool_calls"].([]any); ok {
			for _, rawCall := range toolCalls {
				call, _ := rawCall.(map[string]any)
				function, _ := call["function"].(map[string]any)
				input = append(input, map[string]any{
					"type":      "function_call",
					"id":        call["id"],
					"call_id":   call["id"],
					"name":      function["name"],
					"arguments": function["arguments"],
				})
			}
		}
		if role == "" {
			role = "user"
		}
		if content := responsesMessageContent(message["content"]); len(content) > 0 {
			input = append(input, map[string]any{
				"type":    "message",
				"role":    role,
				"content": content,
			})
		}
	}
	out := map[string]any{
		"model":  model,
		"input":  input,
		"stream": stream,
	}
	for _, key := range []string{"instructions", "temperature", "top_p", "max_output_tokens", "reasoning", "metadata", "previous_response_id", "store", "include"} {
		if value, ok := payload[key]; ok {
			out[key] = value
		}
	}
	if tools, ok := payload["tools"].([]any); ok && len(tools) > 0 {
		out["tools"] = convertChatToolsToResponses(tools)
	}
	if toolChoice, ok := payload["tool_choice"]; ok {
		out["tool_choice"] = toolChoice
	}
	encoded, err := json.Marshal(out)
	if err != nil {
		return body
	}
	return encoded
}

func responsesMessageContent(value any) []any {
	switch typed := value.(type) {
	case string:
		return []any{map[string]any{"type": "input_text", "text": typed}}
	case []any:
		content := make([]any, 0, len(typed))
		for _, raw := range typed {
			part, _ := raw.(map[string]any)
			typ, _ := part["type"].(string)
			switch typ {
			case "text", "input_text":
				content = append(content, map[string]any{"type": "input_text", "text": part["text"]})
			case "image_url", "input_image":
				content = append(content, map[string]any{"type": "input_image", "image_url": part["image_url"]})
			default:
				if text, ok := part["text"].(string); ok {
					content = append(content, map[string]any{"type": "input_text", "text": text})
				}
			}
		}
		return content
	default:
		return nil
	}
}

func convertChatToolsToResponses(tools []any) []any {
	out := make([]any, 0, len(tools))
	for _, raw := range tools {
		tool, _ := raw.(map[string]any)
		if tool["type"] != "function" {
			out = append(out, tool)
			continue
		}
		function, _ := tool["function"].(map[string]any)
		out = append(out, map[string]any{
			"type":        "function",
			"name":        function["name"],
			"description": function["description"],
			"parameters":  function["parameters"],
		})
	}
	return out
}

func fallbackResponsesToChat(body []byte, model string) []byte {
	return fallbackResponsesToChatWithStream(body, model, true)
}

func fallbackResponsesToChatWithStream(body []byte, model string, stream bool) []byte {
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return []byte(fmt.Sprintf(`{"model":%q,"stream":%t,"messages":[{"role":"user","content":""}]}`, model, stream))
	}
	messages := extractChatMessages(payload["input"])
	if len(messages) == 0 {
		messages = []map[string]any{{"role": "user", "content": ""}}
	}
	out := map[string]any{
		"model":    model,
		"stream":   stream,
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

func convertChatToResponses(payload []byte, model string, originalBody []byte, chatBody []byte) []byte {
	converted := responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponsesNonStream(contextOrBackground(nil), model, originalBody, chatBody, payload, nil)
	if len(bytes.TrimSpace(converted)) > 0 {
		return converted
	}
	return payload
}

func writeGrokChatAsResponsesSSE(c *gin.Context, body io.Reader, originalBody []byte, chatBody []byte, model string, wrote *bool) error {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		return fmt.Errorf("streaming not supported")
	}
	c.Header("Content-Type", "text/event-stream")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	c.Status(http.StatusOK)
	*wrote = true
	var state any
	doneSeen := false
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 2*1024*1024)
	for scanner.Scan() {
		line := bytes.TrimSpace(scanner.Bytes())
		if len(line) == 0 {
			continue
		}
		if providerGatewayStreamLineIsDone(line) {
			doneSeen = true
		}
		events := responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponses(
			contextOrBackground(c), model, originalBody, chatBody, line, &state,
		)
		for _, event := range events {
			if len(event) == 0 {
				continue
			}
			if _, err := c.Writer.Write(providerGatewaySSEFrame(event)); err != nil {
				return err
			}
			flusher.Flush()
		}
	}
	if err := scanner.Err(); err != nil {
		return err
	}
	if !doneSeen {
		for _, event := range responsesconverter.CompleteOpenAIChatCompletionsResponseToOpenAIResponses(contextOrBackground(c), chatBody, &state) {
			if len(event) == 0 {
				continue
			}
			if _, err := c.Writer.Write(providerGatewaySSEFrame(event)); err != nil {
				return err
			}
			flusher.Flush()
		}
	}
	return nil
}

func chatSSEData(line []byte) []byte {
	if bytes.HasPrefix(line, []byte("data:")) {
		return bytes.TrimSpace(bytes.TrimPrefix(line, []byte("data:")))
	}
	return line
}

func rewriteRequestStream(body []byte, stream bool) []byte {
	var payload map[string]any
	if json.Unmarshal(body, &payload) != nil || payload == nil {
		return body
	}
	payload["stream"] = stream
	encoded, err := json.Marshal(payload)
	if err != nil {
		return body
	}
	return encoded
}

func buildCompactionRequest(body []byte) []byte {
	var payload map[string]any
	if json.Unmarshal(body, &payload) != nil || payload == nil {
		return body
	}
	payload["stream"] = false
	payload["tools"] = []any{}
	delete(payload, "previous_response_id")
	delete(payload, "client_metadata")
	input := compactionInputItems(payload["input"])
	input = append(input, map[string]any{
		"type": "message",
		"role": "user",
		"content": []any{
			map[string]any{
				"type": "input_text",
				"text": "Summarize the conversation for a coding agent. Preserve user goals, decisions, constraints, files, commands, errors, unresolved work, and exact identifiers. Return only the compact summary.",
			},
		},
	})
	payload["input"] = input
	encoded, err := json.Marshal(payload)
	if err != nil {
		return body
	}
	return encoded
}

func compactionInputItems(value any) []any {
	switch typed := value.(type) {
	case []any:
		return append([]any(nil), typed...)
	case string:
		if strings.TrimSpace(typed) == "" {
			return nil
		}
		return []any{map[string]any{
			"type": "message",
			"role": "user",
			"content": []any{map[string]any{
				"type": "input_text",
				"text": typed,
			}},
		}}
	case map[string]any:
		return []any{typed}
	default:
		return nil
	}
}

func extractChatResponseText(payload []byte) string {
	var parsed map[string]any
	if json.Unmarshal(payload, &parsed) == nil {
		if choices, ok := parsed["choices"].([]any); ok {
			for _, item := range choices {
				choice, _ := item.(map[string]any)
				if message, ok := choice["message"].(map[string]any); ok {
					if text := chatContentText(message["content"]); text != "" {
						return text
					}
				}
				if delta, ok := choice["delta"].(map[string]any); ok {
					if text := chatContentText(delta["content"]); text != "" {
						return text
					}
				}
			}
		}
	}
	var out strings.Builder
	for _, line := range bytes.Split(payload, []byte("\n")) {
		data := chatSSEData(bytes.TrimSpace(line))
		if len(data) == 0 || bytes.Equal(data, []byte("[DONE]")) {
			continue
		}
		var event map[string]any
		if json.Unmarshal(data, &event) != nil {
			continue
		}
		choices, _ := event["choices"].([]any)
		for _, item := range choices {
			choice, _ := item.(map[string]any)
			delta, _ := choice["delta"].(map[string]any)
			out.WriteString(chatContentText(delta["content"]))
		}
	}
	return out.String()
}

func chatContentText(value any) string {
	switch typed := value.(type) {
	case string:
		return typed
	case []any:
		var out strings.Builder
		for _, item := range typed {
			if object, ok := item.(map[string]any); ok {
				if text, ok := object["text"].(string); ok {
					out.WriteString(text)
				}
			}
		}
		return out.String()
	default:
		return ""
	}
}

func writeCompactionResponse(c *gin.Context, model, summary string) {
	encoded := base64.StdEncoding.EncodeToString([]byte(summary))
	itemID := fmt.Sprintf("cmp_%d", time.Now().UnixNano())
	responseID := fmt.Sprintf("resp_%d", time.Now().UnixNano())
	c.JSON(http.StatusOK, gin.H{
		"id":         responseID,
		"object":     "response",
		"created_at": time.Now().Unix(),
		"status":     "completed",
		"model":      model,
		"output": []any{gin.H{
			"type":              "compaction",
			"id":                itemID,
			"encrypted_content": compactSummaryPrefix + encoded,
		}},
		"usage": nil,
	})
}

func expandCompactionItems(body []byte) []byte {
	var payload map[string]any
	if json.Unmarshal(body, &payload) != nil || payload == nil {
		return body
	}
	input, ok := payload["input"].([]any)
	if !ok {
		return body
	}
	changed := false
	for index, item := range input {
		object, ok := item.(map[string]any)
		if !ok || object["type"] != "compaction" {
			continue
		}
		encrypted, _ := object["encrypted_content"].(string)
		if !strings.HasPrefix(encrypted, compactSummaryPrefix) {
			continue
		}
		decoded, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(encrypted, compactSummaryPrefix))
		if err != nil {
			continue
		}
		input[index] = map[string]any{
			"type": "message",
			"role": "assistant",
			"content": []any{map[string]any{
				"type": "input_text",
				"text": string(decoded),
			}},
		}
		changed = true
	}
	if !changed {
		return body
	}
	payload["input"] = input
	encoded, err := json.Marshal(payload)
	if err != nil {
		return body
	}
	return encoded
}

func (s *relayServer) handleBrokerProviderRequest(c *gin.Context, body []byte, route *unifiedGatewayRoute, sourceFormat sdktranslator.Format, fixedAlt string) {
	broker := getGlobalBroker()
	if broker == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "credential broker is not connected", "broker_unavailable")
		return
	}
	provider := unifiedProviderForRoute(s.manifest.UnifiedGateway, route.ProviderID)
	if provider == nil {
		writeAPIError(c, http.StatusBadGateway, "Provider 配置不存在", "provider_not_configured")
		return
	}
	compact := fixedAlt == "responses/compact"
	requestBody := body
	if compact {
		requestBody = buildCompactionRequest(body)
	}
	wireAPI := normalizeUnifiedProviderWireAPI(provider.WireAPI)
	upstreamModel := route.UpstreamModel
	if strings.TrimSpace(upstreamModel) == "" {
		upstreamModel = route.ModelID
	}
	stream := requestBodyStream(requestBody) && !compact
	upstreamBody := rewriteRequestModel(requestBody, upstreamModel)
	if wireAPI == "chat_completions" {
		upstreamBody = convertResponsesToChat(requestBody, upstreamModel, sourceFormat, stream)
	} else if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI) {
		upstreamBody = convertChatToResponsesRequest(requestBody, upstreamModel, stream)
	}
	if stream {
		if wireAPI == "chat_completions" {
			reader, writer := io.Pipe()
			done := make(chan struct{})
			go func() {
				s.writeProviderGatewayChatStream(c, reader, upstreamModel, body, upstreamBody)
				close(done)
			}()
			_, streamErr := broker.ExecuteProviderStream(
				route.ProviderID,
				upstreamModel,
				json.RawMessage(upstreamBody),
				func(chunk []byte) error {
					_, err := writer.Write(chunk)
					return err
				},
			)
			if streamErr != nil {
				_ = writer.CloseWithError(streamErr)
			} else {
				_ = writer.Close()
			}
			<-done
			return
		}
		c.Header("Content-Type", "text/event-stream")
		c.Header("Cache-Control", "no-cache")
		c.Header("Connection", "keep-alive")
		c.Status(http.StatusOK)
		flusher, ok := c.Writer.(http.Flusher)
		if !ok {
			writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
			return
		}
		_, streamErr := broker.ExecuteProviderStream(
			route.ProviderID,
			upstreamModel,
			json.RawMessage(upstreamBody),
			func(chunk []byte) error {
				if _, err := c.Writer.Write(chunk); err != nil {
					return err
				}
				flusher.Flush()
				return nil
			},
		)
		if streamErr != nil {
			writeSSEError(c, streamErr.Error())
		}
		return
	}
	resp, err := broker.ExecuteProvider(route.ProviderID, upstreamModel, json.RawMessage(upstreamBody))
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
	if compact {
		summary := extractProviderResponseText([]byte(payload), wireAPI)
		if strings.TrimSpace(summary) == "" {
			writeAPIError(c, http.StatusBadGateway, "Provider 压缩响应没有摘要内容", "provider_compact_empty")
			return
		}
		writeCompactionResponse(c, route.ModelID, summary)
		return
	}
	if wireAPI == "chat_completions" && status >= 200 && status < 300 && sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse) {
		payload = string(convertChatToResponses([]byte(payload), upstreamModel, body, upstreamBody))
		contentType = "application/json"
	}
	c.Data(status, contentType, []byte(payload))
}

func unifiedProviderForRoute(cfg *unifiedGatewayConfig, providerID string) *unifiedProviderSpec {
	if cfg == nil {
		return nil
	}
	for index := range cfg.Providers {
		if strings.EqualFold(strings.TrimSpace(cfg.Providers[index].ID), strings.TrimSpace(providerID)) {
			return &cfg.Providers[index]
		}
	}
	return nil
}

func normalizeUnifiedProviderWireAPI(value string) string {
	if strings.Contains(strings.ToLower(strings.TrimSpace(value)), "chat") {
		return "chat_completions"
	}
	return "responses"
}

func extractProviderResponseText(payload []byte, wireAPI string) string {
	if wireAPI == "chat_completions" {
		return extractChatResponseText(payload)
	}
	var response map[string]any
	if json.Unmarshal(payload, &response) != nil {
		return ""
	}
	if outputText, ok := response["output_text"].(string); ok && strings.TrimSpace(outputText) != "" {
		return outputText
	}
	if output, ok := response["output"].([]any); ok {
		var text strings.Builder
		for _, item := range output {
			object, _ := item.(map[string]any)
			if content, ok := object["content"].([]any); ok {
				for _, part := range content {
					partObject, _ := part.(map[string]any)
					if value, ok := partObject["text"].(string); ok {
						text.WriteString(value)
					}
				}
			}
		}
		return text.String()
	}
	return ""
}
