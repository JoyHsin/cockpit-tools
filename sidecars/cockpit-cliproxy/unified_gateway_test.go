package main

import (
	"bytes"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func TestStripUnifiedGatewayPrefix(t *testing.T) {
	path, ok := stripUnifiedGatewayPrefix("/_cockpit-ugw/abc123/v1/responses", "abc123")
	if !ok || path != "/v1/responses" {
		t.Fatalf("got path=%q ok=%v", path, ok)
	}
	if _, ok := stripUnifiedGatewayPrefix("/_cockpit-ugw/other/v1/models", "abc123"); ok {
		t.Fatal("expected capability mismatch")
	}
}

func TestOfficialRouteNeverMatchesGrok(t *testing.T) {
	cfg := &unifiedGatewayConfig{
		Routes: []unifiedGatewayRoute{
			{ModelID: "gpt-5.4", ProviderID: "official-codex", Route: "official", UpstreamModel: "gpt-5.4"},
			{ModelID: "grok-4.5", ProviderID: "grok-oauth", Route: "grok-oauth", UpstreamModel: "grok-4.5"},
		},
	}
	official := unifiedRouteForModel(cfg, "gpt-5.4")
	if official == nil || official.Route != unifiedOfficialRoute {
		t.Fatal("official model must stay on official route")
	}
	grok := unifiedRouteForModel(cfg, "grok-4.5")
	if grok == nil || grok.Route != unifiedGrokRoute {
		t.Fatal("grok model must stay on grok route")
	}
	if unifiedRouteForModel(cfg, "unknown-model") != nil {
		t.Fatal("unknown models must not silently route")
	}
}

func TestGrokPoolOrdersAffinityThenPriority(t *testing.T) {
	members := []unifiedGrokPoolMember{
		{AccountID: "b", Priority: 2},
		{AccountID: "a", Priority: 1},
		{AccountID: "c", Priority: 3, BackupOnly: true},
	}
	cfg := &unifiedGatewayConfig{GrokPool: members}
	ordered := grokMembersForRoute(cfg, "grok-4.5")
	if ordered[0].AccountID != "a" || ordered[len(ordered)-1].AccountID != "c" {
		t.Fatalf("unexpected order: %+v", ordered)
	}
	withAffinity := orderGrokMembers(ordered, "b")
	if withAffinity[0].AccountID != "b" {
		t.Fatalf("affinity should win: %+v", withAffinity)
	}
}

func TestCanRetryGrokOnlyBeforeFirstByte(t *testing.T) {
	if canRetryGrokError(nil, false, 0, 2) {
		t.Fatal("nil error should not retry")
	}
	if !canRetryGrokError(errString("401 unauthorized"), false, 0, 2) {
		t.Fatal("401 before first byte should retry")
	}
	if canRetryGrokError(errString("401 unauthorized"), true, 0, 2) {
		t.Fatal("must not switch accounts after first byte")
	}
}

type errString string

func (e errString) Error() string { return string(e) }

func TestOfficialAuthRequired(t *testing.T) {
	if officialAuthPresent(http.Header{}) {
		t.Fatal("empty headers must fail official auth")
	}
	header := http.Header{}
	header.Set("Authorization", "Bearer official-token")
	if !officialAuthPresent(header) {
		t.Fatal("bearer token should pass")
	}
}

func TestUnifiedUnknownModelReturns404(t *testing.T) {
	gin.SetMode(gin.TestMode)
	relay := &relayServer{
		manifest: &manifest{
			UnifiedGateway: &unifiedGatewayConfig{
				Enabled:         true,
				CapabilityToken: "tok",
				Routes:          []unifiedGatewayRoute{{ModelID: "gpt-5.4", Route: "official", ProviderID: "official-codex"}},
			},
		},
	}
	router := gin.New()
	router.Use(relay.unifiedGatewayMiddleware())
	router.POST("/v1/responses", func(c *gin.Context) {
		relay.tryHandleUnifiedRequest(c, []byte(`{"model":"missing"}`), sdktranslator.FormatOpenAIResponse, "")
	})
	req := httptest.NewRequest(http.MethodPost, "/_cockpit-ugw/tok/v1/responses", bytes.NewReader([]byte(`{"model":"missing"}`)))
	rec := httptest.NewRecorder()
	router.ServeHTTP(rec, req)
	if rec.Code != http.StatusNotFound {
		t.Fatalf("status=%d body=%s", rec.Code, rec.Body.String())
	}
}

func TestBrokerHMACMatchesRustPayload(t *testing.T) {
	key := bytes.Repeat([]byte{7}, 32)
	payload := []byte("1|get_grok_access_token|acc-1")
	got := signBroker(key, payload)
	mac := hmac.New(sha256.New, key)
	_, _ = mac.Write(payload)
	want := hex.EncodeToString(mac.Sum(nil))
	if got != want {
		t.Fatalf("hmac mismatch")
	}
}

func TestFallbackResponsesToChatKeepsTools(t *testing.T) {
	body := []byte(`{"model":"grok-4.5","input":[{"role":"user","content":[{"type":"input_text","text":"hi"}]}],"tools":[{"type":"function","name":"search"}]}`)
	out := fallbackResponsesToChat(body, "grok-4.5")
	var parsed map[string]any
	if err := json.Unmarshal(out, &parsed); err != nil {
		t.Fatal(err)
	}
	if parsed["model"] != "grok-4.5" {
		t.Fatalf("model=%v", parsed["model"])
	}
	if parsed["tools"] == nil {
		t.Fatal("tools should be preserved")
	}
}
