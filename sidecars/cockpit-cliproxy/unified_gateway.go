package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

const (
	unifiedGatewayPrefix    = "/_cockpit-ugw/"
	unifiedOfficialRoute    = "official"
	unifiedGrokRoute        = "grok-oauth"
	unifiedOfficialProvider = "official-codex"
	unifiedGrokProvider     = "grok-oauth"
	defaultOfficialUpstream = "https://chatgpt.com/backend-api/codex"
)

type unifiedGatewayConfig struct {
	Enabled             bool                    `json:"enabled"`
	ProtocolVersion     uint32                  `json:"protocolVersion"`
	CapabilityToken     string                  `json:"capabilityToken"`
	LanMode             bool                    `json:"lanMode"`
	OfficialPassthrough bool                    `json:"officialPassthrough"`
	OfficialUpstream    string                  `json:"officialUpstream"`
	BrokerSocket        string                  `json:"brokerSocket"`
	Routes              []unifiedGatewayRoute   `json:"routes"`
	GrokPool            []unifiedGrokPoolMember `json:"grokPool"`
	Providers           []unifiedProviderSpec   `json:"providers"`
}

type unifiedGatewayRoute struct {
	ModelID       string         `json:"modelId"`
	ProviderID    string         `json:"providerId"`
	Route         string         `json:"route"`
	UpstreamModel string         `json:"upstreamModel"`
	Capabilities  map[string]any `json:"capabilities"`
}

type unifiedGrokPoolMember struct {
	AccountID           string   `json:"accountId"`
	Priority            int      `json:"priority"`
	Weight              uint32   `json:"weight"`
	BackupOnly          bool     `json:"backupOnly"`
	MinRemainingPercent *int     `json:"minRemainingPercent"`
	AllowedModels       []string `json:"allowedModels"`
}

type unifiedProviderSpec struct {
	ID               string   `json:"id"`
	Type             string   `json:"type"`
	BaseURL          string   `json:"baseUrl"`
	WireAPI          string   `json:"wireApi"`
	CredentialRefIDs []string `json:"credentialRefIds"`
}

type unifiedSessionAffinity struct {
	mu      sync.Mutex
	binding map[string]string
}

func newUnifiedSessionAffinity() *unifiedSessionAffinity {
	return &unifiedSessionAffinity{binding: map[string]string{}}
}

func (s *unifiedSessionAffinity) get(sessionID string) string {
	if s == nil || sessionID == "" {
		return ""
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.binding[sessionID]
}

func (s *unifiedSessionAffinity) set(sessionID, accountID string) {
	if s == nil || sessionID == "" || accountID == "" {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.binding[sessionID] = accountID
}

func (m *manifest) unifiedGatewayEnabled() bool {
	return m != nil && m.UnifiedGateway != nil && m.UnifiedGateway.Enabled
}

func stripUnifiedGatewayPrefix(path, token string) (string, bool) {
	prefix := unifiedGatewayPrefix + strings.TrimSpace(token)
	if !strings.HasPrefix(path, prefix) {
		return path, false
	}
	rest := strings.TrimPrefix(path, prefix)
	if rest == "" {
		return "/", true
	}
	if !strings.HasPrefix(rest, "/") {
		rest = "/" + rest
	}
	return rest, true
}

func unifiedRouteForModel(cfg *unifiedGatewayConfig, model string) *unifiedGatewayRoute {
	if cfg == nil {
		return nil
	}
	wanted := strings.TrimSpace(model)
	for i := range cfg.Routes {
		if strings.EqualFold(strings.TrimSpace(cfg.Routes[i].ModelID), wanted) {
			return &cfg.Routes[i]
		}
	}
	return nil
}

func officialAuthPresent(header http.Header) bool {
	if header == nil {
		return false
	}
	auth := strings.TrimSpace(header.Get("Authorization"))
	return strings.HasPrefix(strings.ToLower(auth), "bearer ") && len(auth) > 8
}

func (s *relayServer) unifiedGatewayMiddleware() gin.HandlerFunc {
	return func(c *gin.Context) {
		if s == nil || s.manifest == nil || !s.manifest.unifiedGatewayEnabled() || c.Request == nil || c.Request.URL == nil {
			c.Next()
			return
		}
		cfg := s.manifest.UnifiedGateway
		rewritten, ok := stripUnifiedGatewayPrefix(c.Request.URL.Path, cfg.CapabilityToken)
		if !ok {
			if strings.HasPrefix(c.Request.URL.Path, unifiedGatewayPrefix) {
				writeAPIError(c, http.StatusNotFound, "unknown unified gateway capability path", "not_found")
				c.Abort()
				return
			}
			c.Next()
			return
		}
		c.Request.URL.Path = rewritten
		c.Request.URL.RawPath = rewritten
		c.Set("unifiedGatewayAuthorized", true)
		if isModelsRequest(c.Request) {
			s.handleUnifiedModels(c)
			c.Abort()
			return
		}
		c.Next()
	}
}

func (s *relayServer) handleUnifiedHealthz(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{"ok": true, "service": "unified-gateway"})
}

func (s *relayServer) handleUnifiedModels(c *gin.Context) {
	if s == nil || s.manifest == nil || s.manifest.UnifiedGateway == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "unified gateway is not configured", "service_unavailable")
		return
	}
	ids := make([]string, 0, len(s.manifest.UnifiedGateway.Routes))
	for _, route := range s.manifest.UnifiedGateway.Routes {
		if strings.TrimSpace(route.ModelID) != "" {
			ids = append(ids, route.ModelID)
		}
	}
	if isCodexClientModelsRequest(c.Request) {
		c.JSON(http.StatusOK, buildCodexClientModelsResponse(ids, nil, nil))
		return
	}
	c.JSON(http.StatusOK, buildModelsResponse(ids))
}

func (s *relayServer) tryHandleUnifiedRequest(c *gin.Context, body []byte, sourceFormat sdktranslator.Format, fixedAlt string) bool {
	if s == nil || s.manifest == nil || !s.manifest.unifiedGatewayEnabled() {
		return false
	}
	if _, ok := c.Get("unifiedGatewayAuthorized"); !ok {
		return false
	}
	cfg := s.manifest.UnifiedGateway
	model := requestBodyModel(body)
	route := unifiedRouteForModel(cfg, model)
	if route == nil {
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("unknown model %s", model), "model_not_found")
		return true
	}
	if cfg.LanMode && route.Route != unifiedOfficialRoute {
		writeAPIError(c, http.StatusForbidden, "LAN mode does not expose credentialed external models", "lan_external_blocked")
		return true
	}
	switch {
	case route.Route == unifiedOfficialRoute || route.ProviderID == unifiedOfficialProvider:
		if !officialAuthPresent(c.Request.Header) {
			writeAPIError(c, http.StatusUnauthorized, "official Codex authentication is required", "official_auth_invalid")
			return true
		}
		s.handleOfficialPassthrough(c, body, route)
	case route.Route == unifiedGrokRoute || route.ProviderID == unifiedGrokProvider:
		s.handleGrokOAuthRequest(c, body, route, sourceFormat, fixedAlt)
	default:
		s.handleBrokerProviderRequest(c, body, route)
	}
	return true
}

func requestSessionID(body []byte, header http.Header) string {
	var payload struct {
		ID        string `json:"id"`
		SessionID string `json:"session_id"`
	}
	_ = json.Unmarshal(body, &payload)
	if id := strings.TrimSpace(payload.ID); id != "" {
		return id
	}
	if id := strings.TrimSpace(payload.SessionID); id != "" {
		return id
	}
	if header == nil {
		return ""
	}
	for _, name := range []string{"Session-Id", "X-Session-ID", "session_id"} {
		if value := strings.TrimSpace(header.Get(name)); value != "" {
			return value
		}
	}
	return ""
}

func rewriteRequestModel(body []byte, model string) []byte {
	var payload map[string]json.RawMessage
	if err := json.Unmarshal(body, &payload); err != nil || payload == nil {
		return body
	}
	encoded, err := json.Marshal(model)
	if err != nil {
		return body
	}
	payload["model"] = encoded
	next, err := json.Marshal(payload)
	if err != nil {
		return body
	}
	return next
}

func copyHeaderAllowlist(dst, src http.Header, names ...string) {
	if dst == nil || src == nil {
		return
	}
	for _, name := range names {
		if value := strings.TrimSpace(src.Get(name)); value != "" {
			dst.Set(name, value)
		}
	}
}

func writeSSEError(c *gin.Context, message string) {
	if c.Writer.Written() {
		_, _ = io.WriteString(c.Writer, fmt.Sprintf("event: error\ndata: {\"error\":{\"message\":%q}}\n\n", message))
		if flusher, ok := c.Writer.(http.Flusher); ok {
			flusher.Flush()
		}
		return
	}
	writeAPIError(c, http.StatusBadGateway, message, "upstream_error")
}

func streamHTTPResponse(c *gin.Context, resp *http.Response) error {
	defer resp.Body.Close()
	for key, values := range resp.Header {
		lower := strings.ToLower(key)
		if lower == "authorization" || lower == "cookie" || lower == "set-cookie" {
			continue
		}
		for _, value := range values {
			c.Writer.Header().Add(key, value)
		}
	}
	status := resp.StatusCode
	if status <= 0 {
		status = http.StatusOK
	}
	c.Status(status)
	_, err := io.Copy(c.Writer, resp.Body)
	if flusher, ok := c.Writer.(http.Flusher); ok {
		flusher.Flush()
	}
	return err
}

func contextOrBackground(c *gin.Context) context.Context {
	if c == nil || c.Request == nil {
		return context.Background()
	}
	return c.Request.Context()
}

func bytesReader(body []byte) io.ReadCloser {
	return io.NopCloser(bytes.NewReader(body))
}
