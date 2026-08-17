package main

import (
	"fmt"
	"net/http"
	"strings"

	"github.com/gin-gonic/gin"
)

func officialUpstreamURL(cfg *unifiedGatewayConfig, path string) string {
	base := defaultOfficialUpstream
	if cfg != nil && strings.TrimSpace(cfg.OfficialUpstream) != "" {
		base = strings.TrimRight(strings.TrimSpace(cfg.OfficialUpstream), "/")
	}
	if path == "" {
		path = "/responses"
	}
	if !strings.HasPrefix(path, "/") {
		path = "/" + path
	}
	return base + path
}

func (s *relayServer) handleOfficialPassthrough(c *gin.Context, body []byte, route *unifiedGatewayRoute) {
	cfg := s.manifest.UnifiedGateway
	upstreamPath := "/responses"
	if strings.Contains(c.Request.URL.Path, "compact") {
		upstreamPath = "/responses/compact"
	}
	req, err := http.NewRequestWithContext(contextOrBackground(c), http.MethodPost, officialUpstreamURL(cfg, upstreamPath), bytesReader(body))
	if err != nil {
		writeAPIError(c, http.StatusBadGateway, "failed to build official Codex request", "official_upstream_failed")
		return
	}
	copyHeaderAllowlist(req.Header, c.Request.Header,
		"Authorization",
		"Chatgpt-Account-Id",
		"Content-Type",
		"Accept",
		"Originator",
		"User-Agent",
		"Version",
		"Session-Id",
		"X-Session-ID",
		"X-Client-Request-Id",
		"X-Openai-Actor-Authorization",
		"OpenAI-Beta",
	)
	if req.Header.Get("Content-Type") == "" {
		req.Header.Set("Content-Type", "application/json")
	}
	if req.Header.Get("Originator") == "" {
		req.Header.Set("Originator", "codex_cli_rs")
	}
	upstreamBody := body
	if route != nil && strings.TrimSpace(route.UpstreamModel) != "" {
		upstreamBody = rewriteRequestModel(body, route.UpstreamModel)
	}
	req.Body = bytesReader(upstreamBody)
	req.ContentLength = int64(len(upstreamBody))
	client := &http.Client{Timeout: 0}
	resp, err := client.Do(req)
	if err != nil {
		writeSSEError(c, fmt.Sprintf("official Codex upstream failed: %v", err))
		return
	}
	if err := streamHTTPResponse(c, resp); err != nil && c.Writer.Written() {
		writeSSEError(c, "official Codex stream terminated")
	}
}
