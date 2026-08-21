package main

import (
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
)

var officialResponsesUpgrader = websocket.Upgrader{
	ReadBufferSize:  4096,
	WriteBufferSize: 4096,
	CheckOrigin: func(*http.Request) bool {
		return true
	},
}

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

func officialWebsocketURL(cfg *unifiedGatewayConfig, req *http.Request) (string, error) {
	parsed, err := url.Parse(officialUpstreamURL(cfg, "/responses"))
	if err != nil {
		return "", err
	}
	switch strings.ToLower(parsed.Scheme) {
	case "http":
		parsed.Scheme = "ws"
	case "https":
		parsed.Scheme = "wss"
	default:
		return "", fmt.Errorf("unsupported official websocket scheme %q", parsed.Scheme)
	}
	if strings.TrimSpace(parsed.Host) == "" {
		return "", fmt.Errorf("official websocket host is empty")
	}
	if req != nil && req.URL != nil {
		parsed.RawQuery = req.URL.RawQuery
	}
	return parsed.String(), nil
}

func (s *relayServer) handleOfficialResponsesWebsocket(c *gin.Context) {
	if c == nil || c.Request == nil {
		return
	}
	client, err := officialResponsesUpgrader.Upgrade(c.Writer, c.Request, nil)
	if err != nil {
		return
	}
	defer client.Close()

	messageType, first, err := client.ReadMessage()
	if err != nil {
		return
	}
	for requestBodyModel(first) == "" && websocketEnvelopeModel(first) == "" {
		messageType, first, err = client.ReadMessage()
		if err != nil {
			return
		}
	}
	model := websocketEnvelopeModel(first)
	route := unifiedRouteForModel(s.manifest.UnifiedGateway, model)
	if route != nil && (route.Route == unifiedGrokRoute || route.ProviderID == unifiedGrokProvider) {
		if err := s.executeGrokWebsocketTurn(client, first, route); err != nil {
			_ = client.WriteJSON(gin.H{"type": "error", "error": gin.H{"message": err.Error(), "code": "grok_upstream_failed"}})
		}
		for {
			_, payload, err := client.ReadMessage()
			if err != nil {
				return
			}
			if err := s.executeGrokWebsocketTurn(client, payload, route); err != nil {
				_ = client.WriteJSON(gin.H{"type": "error", "error": gin.H{"message": err.Error(), "code": "grok_upstream_failed"}})
			}
		}
	}

	if !officialAuthPresent(c.Request.Header) {
		_ = client.WriteJSON(gin.H{"type": "error", "error": gin.H{"message": "official Codex authentication is required", "code": "official_auth_invalid"}})
		return
	}
	upstreamURL, err := officialWebsocketURL(s.manifest.UnifiedGateway, c.Request)
	if err != nil {
		_ = client.WriteJSON(gin.H{"type": "error", "error": gin.H{"message": err.Error(), "code": "official_upstream_failed"}})
		return
	}
	hdr := http.Header{}
	copyHeaderAllowlist(hdr, c.Request.Header,
		"Authorization",
		"Chatgpt-Account-Id",
		"Originator",
		"User-Agent",
		"Version",
		"Session-Id",
		"X-Session-ID",
		"X-Client-Request-Id",
		"X-Openai-Actor-Authorization",
		"OpenAI-Beta",
		"Cookie",
	)
	dialer := websocket.Dialer{
		Proxy:             http.ProxyFromEnvironment,
		HandshakeTimeout:  15 * time.Second,
		EnableCompression: true,
	}
	upstream, resp, err := dialer.DialContext(contextOrBackground(c), upstreamURL, hdr)
	if resp != nil && resp.Body != nil {
		_, _ = io.Copy(io.Discard, resp.Body)
		_ = resp.Body.Close()
	}
	if err != nil {
		_ = client.WriteJSON(gin.H{"type": "error", "error": gin.H{"message": fmt.Sprintf("official Codex websocket failed: %v", err), "code": "official_upstream_failed"}})
		return
	}
	defer upstream.Close()
	if err := upstream.WriteMessage(messageType, first); err != nil {
		return
	}
	errc := make(chan error, 2)
	go proxyWebsocket(client, upstream, errc)
	go proxyWebsocket(upstream, client, errc)
	<-errc
}

func proxyWebsocket(src, dst *websocket.Conn, errc chan<- error) {
	for {
		messageType, payload, err := src.ReadMessage()
		if err != nil {
			errc <- err
			return
		}
		if err := dst.WriteMessage(messageType, payload); err != nil {
			errc <- err
			return
		}
	}
}
