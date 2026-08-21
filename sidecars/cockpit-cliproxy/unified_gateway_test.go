package main

import (
	"bytes"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func TestWebsocketPrewarmDetection(t *testing.T) {
	if !isWebsocketPrewarm([]byte(`{"type":"response.create","model":"grok-4.5","generate":false}`)) {
		t.Fatal("generate:false should be prewarm")
	}
	if isWebsocketPrewarm([]byte(`{"type":"response.create","model":"grok-4.5","generate":true}`)) {
		t.Fatal("generate:true is a real turn")
	}
	if isWebsocketPrewarm([]byte(`{"type":"response.create","model":"grok-4.5"}`)) {
		t.Fatal("missing generate is a real turn")
	}
}

func TestWebsocketEnvelopeModel(t *testing.T) {
	if got := websocketEnvelopeModel([]byte(`{"type":"response.create","model":"grok-4.5"}`)); got != "grok-4.5" {
		t.Fatalf("got %q", got)
	}
	if got := websocketEnvelopeModel([]byte(`{"type":"response.create","response":{"model":"gpt-5.6-terra"}}`)); got != "gpt-5.6-terra" {
		t.Fatalf("nested got %q", got)
	}
}

func TestWebsocketMessageStripsCreateType(t *testing.T) {
	out := websocketMessageToResponsesBody([]byte(`{"type":"response.create","model":"grok-4.5","input":"hi"}`))
	if strings.Contains(string(out), `"type":"response.create"`) {
		t.Fatalf("type should be stripped: %s", out)
	}
	if requestBodyModel(out) != "grok-4.5" {
		t.Fatalf("model=%q body=%s", requestBodyModel(out), out)
	}
}

func TestOfficialWebsocketURLUsesWSS(t *testing.T) {
	got, err := officialWebsocketURL(&unifiedGatewayConfig{OfficialUpstream: "https://chatgpt.com/backend-api/codex"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if got != "wss://chatgpt.com/backend-api/codex/responses" {
		t.Fatalf("url=%q", got)
	}
}

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
			{ModelID: "grok-oauth/grok-4.5", ProviderID: "grok-oauth", Route: "grok-oauth", UpstreamModel: "grok-4.5"},
		},
	}
	official := unifiedRouteForModel(cfg, "gpt-5.4")
	if official == nil || official.Route != unifiedOfficialRoute {
		t.Fatal("official model must stay on official route")
	}
	grok := unifiedRouteForModel(cfg, "grok-oauth/grok-4.5")
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
	ordered := grokMembersForRoute(cfg, "grok-oauth/grok-4.5", "grok-4.5")
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
		body, _ := io.ReadAll(c.Request.Body)
		relay.tryHandleUnifiedRequest(c, body, sdktranslator.FormatOpenAIResponse, "")
	})
	router.NoRoute(func(c *gin.Context) {
		c.JSON(http.StatusNotFound, gin.H{"error": "endpoint not supported"})
	})
	req := httptest.NewRequest(http.MethodPost, "/_cockpit-ugw/tok/v1/responses", bytes.NewReader([]byte(`{"model":"missing"}`)))
	rec := httptest.NewRecorder()
	wrapUnifiedGatewayHandler(relay.manifest, router).ServeHTTP(rec, req)
	if rec.Code != http.StatusNotFound {
		t.Fatalf("status=%d body=%s", rec.Code, rec.Body.String())
	}
	if strings.Contains(rec.Body.String(), "endpoint not supported") {
		t.Fatalf("capability URL hit NoRoute instead of model handler: %s", rec.Body.String())
	}
}

func TestUnifiedGatewayRewritesPathBeforeRouting(t *testing.T) {
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
	engine := gin.New()
	engine.Use(relay.unifiedGatewayMiddleware())
	hit := false
	engine.POST("/v1/responses", func(c *gin.Context) {
		hit = true
		if _, ok := c.Get("unifiedGatewayAuthorized"); !ok {
			t.Fatal("missing unified gateway auth flag")
		}
		c.JSON(http.StatusOK, gin.H{"ok": true})
	})
	engine.NoRoute(func(c *gin.Context) {
		c.JSON(http.StatusNotFound, gin.H{"error": "endpoint not supported"})
	})
	req := httptest.NewRequest(http.MethodPost, "/_cockpit-ugw/tok/v1/responses", bytes.NewReader([]byte(`{"model":"gpt-5.4"}`)))
	rec := httptest.NewRecorder()
	wrapUnifiedGatewayHandler(relay.manifest, engine).ServeHTTP(rec, req)
	if !hit || rec.Code != http.StatusOK {
		t.Fatalf("hit=%v status=%d body=%s", hit, rec.Code, rec.Body.String())
	}
}

func TestUnifiedGatewayWrongTokenStaysUnsupported(t *testing.T) {
	gin.SetMode(gin.TestMode)
	relay := &relayServer{
		manifest: &manifest{
			UnifiedGateway: &unifiedGatewayConfig{
				Enabled:         true,
				CapabilityToken: "tok",
			},
		},
	}
	engine := gin.New()
	engine.Use(relay.unifiedGatewayMiddleware())
	engine.POST("/v1/responses", func(c *gin.Context) {
		c.JSON(http.StatusOK, gin.H{"ok": true})
	})
	engine.NoRoute(func(c *gin.Context) {
		writeAPIError(c, http.StatusNotFound, "endpoint not supported", "not_found")
	})
	req := httptest.NewRequest(http.MethodPost, "/_cockpit-ugw/other/v1/responses", bytes.NewReader([]byte(`{}`)))
	rec := httptest.NewRecorder()
	wrapUnifiedGatewayHandler(relay.manifest, engine).ServeHTTP(rec, req)
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

func TestCompactionRequestPreservesStringInputAndDisablesTools(t *testing.T) {
	body := []byte(`{"model":"grok-oauth/grok-4.5","input":"existing history","tools":[{"type":"function","name":"search"}],"stream":true}`)
	var parsed map[string]any
	if err := json.Unmarshal(buildCompactionRequest(body), &parsed); err != nil {
		t.Fatal(err)
	}
	if parsed["stream"] != false {
		t.Fatalf("stream=%v", parsed["stream"])
	}
	tools, ok := parsed["tools"].([]any)
	if !ok || len(tools) != 0 {
		t.Fatalf("tools=%v", parsed["tools"])
	}
	input, ok := parsed["input"].([]any)
	if !ok || len(input) != 2 {
		t.Fatalf("input=%v", parsed["input"])
	}
	if !strings.Contains(string(buildCompactionRequest(body)), "existing history") {
		t.Fatal("string input was dropped")
	}
}

func TestCompactionResponseRoundTrip(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	ctx, _ := gin.CreateTestContext(recorder)
	writeCompactionResponse(ctx, "grok-oauth/grok-4.5", "keep this summary")
	var response map[string]any
	if err := json.Unmarshal(recorder.Body.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	output, ok := response["output"].([]any)
	if !ok || len(output) != 1 {
		t.Fatalf("output=%v", response["output"])
	}
	inputBody, _ := json.Marshal(map[string]any{"input": output})
	expanded := expandCompactionItems(inputBody)
	if strings.Contains(string(expanded), "encrypted_content") || !strings.Contains(string(expanded), "keep this summary") {
		t.Fatalf("expanded=%s", expanded)
	}
}

func TestGrokChatStreamConvertsDoneAndTextEvents(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	ctx, _ := gin.CreateTestContext(recorder)
	stream := strings.NewReader("data: {\"id\":\"chatcmpl-test\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n")
	wrote := false
	if err := writeGrokChatAsResponsesSSE(ctx, stream, []byte(`{"model":"grok-oauth/grok-4.5"}`), []byte(`{"model":"grok-4.5","stream":true}`), "grok-4.5", &wrote); err != nil {
		t.Fatal(err)
	}
	body := recorder.Body.String()
	if !wrote || !strings.Contains(body, "hello") {
		t.Fatalf("wrote=%v body=%s", wrote, body)
	}
	if !strings.Contains(body, "response.completed") {
		t.Fatalf("missing terminal response event: %s", body)
	}
}

func TestProviderResponseTextSupportsOutputText(t *testing.T) {
	if got := extractProviderResponseText([]byte(`{"output_text":"summary"}`), "responses"); got != "summary" {
		t.Fatalf("got %q", got)
	}
}

func TestChatRequestConvertsToResponsesInput(t *testing.T) {
	out := convertChatToResponsesRequest([]byte(`{"model":"deepseek-chat","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}],"stream":true}`), "provider/model", true)
	var parsed map[string]any
	if err := json.Unmarshal(out, &parsed); err != nil {
		t.Fatal(err)
	}
	if parsed["model"] != "provider/model" || parsed["stream"] != true {
		t.Fatalf("header=%v", parsed)
	}
	input, ok := parsed["input"].([]any)
	if !ok || len(input) != 1 || !strings.Contains(string(out), "input_text") {
		t.Fatalf("input=%v body=%s", parsed["input"], out)
	}
	if !strings.Contains(string(out), `"name":"lookup"`) {
		t.Fatalf("tools=%s", out)
	}
}
