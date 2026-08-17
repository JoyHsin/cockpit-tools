package main

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"os"
	"runtime"
	"sync"
	"time"
)

const (
	brokerHandshakeBytes = 49
	brokerMaxFrame       = 8 << 20
	brokerProtocol       = 1
)

type brokerSecrets struct {
	sessionKey []byte
	nonce      []byte
}

type brokerClient struct {
	conn   net.Conn
	key    []byte
	seq    uint64
	mu     sync.Mutex
	closed bool
}

func readBrokerHandshake(r io.Reader) (*brokerSecrets, error) {
	buf := make([]byte, brokerHandshakeBytes)
	if _, err := io.ReadFull(r, buf); err != nil {
		return nil, fmt.Errorf("read broker handshake: %w", err)
	}
	if buf[0] != 1 {
		return nil, fmt.Errorf("unsupported broker handshake version %d", buf[0])
	}
	return &brokerSecrets{
		sessionKey: append([]byte(nil), buf[1:33]...),
		nonce:      append([]byte(nil), buf[33:49]...),
	}, nil
}

func signBroker(key, payload []byte) string {
	mac := hmac.New(sha256.New, key)
	_, _ = mac.Write(payload)
	return hex.EncodeToString(mac.Sum(nil))
}

func writeBrokerFrame(w io.Writer, payload []byte) error {
	if len(payload) == 0 || len(payload) > brokerMaxFrame {
		return fmt.Errorf("invalid broker frame length %d", len(payload))
	}
	var header [4]byte
	binary.LittleEndian.PutUint32(header[:], uint32(len(payload)))
	if _, err := w.Write(header[:]); err != nil {
		return err
	}
	_, err := w.Write(payload)
	return err
}

func readBrokerFrame(r io.Reader) ([]byte, error) {
	var header [4]byte
	if _, err := io.ReadFull(r, header[:]); err != nil {
		return nil, err
	}
	length := binary.LittleEndian.Uint32(header[:])
	if length == 0 || length > brokerMaxFrame {
		return nil, fmt.Errorf("invalid broker frame length %d", length)
	}
	payload := make([]byte, length)
	if _, err := io.ReadFull(r, payload); err != nil {
		return nil, err
	}
	return payload, nil
}

func dialBroker(socketPath string) (net.Conn, error) {
	if runtime.GOOS == "windows" {
		user := os.Getenv("USERNAME")
		if user == "" {
			user = "user"
		}
		return net.DialTimeout("unix", `\\.\pipe\cockpit-ugw-broker-`+user, 3*time.Second)
	}
	return net.DialTimeout("unix", socketPath, 3*time.Second)
}

func connectBroker(socketPath string, secrets *brokerSecrets) (*brokerClient, error) {
	var conn net.Conn
	var err error
	for attempt := 0; attempt < 20; attempt++ {
		conn, err = dialBroker(socketPath)
		if err == nil {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	if err != nil {
		return nil, err
	}
	nonceHex := hex.EncodeToString(secrets.nonce)
	pid := os.Getpid()
	payload := []byte(fmt.Sprintf("hello|%d|%d|%s", brokerProtocol, pid, nonceHex))
	hello := map[string]any{
		"protocolVersion": brokerProtocol,
		"childPid":        pid,
		"nonce":           nonceHex,
		"hmac":            signBroker(secrets.sessionKey, payload),
	}
	raw, err := json.Marshal(hello)
	if err != nil {
		_ = conn.Close()
		return nil, err
	}
	if err := writeBrokerFrame(conn, raw); err != nil {
		_ = conn.Close()
		return nil, err
	}
	response, err := readBrokerFrame(conn)
	if err != nil {
		_ = conn.Close()
		return nil, err
	}
	var parsed struct {
		Type string `json:"type"`
	}
	if err := json.Unmarshal(response, &parsed); err != nil || parsed.Type != "hello_ok" {
		_ = conn.Close()
		return nil, fmt.Errorf("broker handshake rejected")
	}
	return &brokerClient{conn: conn, key: secrets.sessionKey, seq: 1}, nil
}

func (c *brokerClient) call(kind, bodyKey, body string, extra map[string]any) (map[string]any, error) {
	if c == nil {
		return nil, fmt.Errorf("broker is not connected")
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return nil, fmt.Errorf("broker connection closed")
	}
	seq := c.seq
	c.seq++
	payload := []byte(fmt.Sprintf("%d|%s|%s", seq, kind, body))
	req := map[string]any{
		"type": kind,
		"seq":  seq,
		"hmac": signBroker(c.key, payload),
	}
	if bodyKey != "" {
		req[bodyKey] = json.RawMessage([]byte(strconvQuote(body)))
	}
	for key, value := range extra {
		req[key] = value
	}
	// Prefer structured fields over the quoted helper when extras already include them.
	if kind == "get_grok_access_token" {
		req["type"] = "get_grok_access_token"
		req["accountId"] = extra["accountId"]
	}
	if kind == "mark_grok_account" {
		req["type"] = "mark_grok_account"
	}
	if kind == "execute_provider" {
		req["type"] = "execute_provider_request"
	}
	raw, err := json.Marshal(req)
	if err != nil {
		return nil, err
	}
	if err := writeBrokerFrame(c.conn, raw); err != nil {
		return nil, err
	}
	resp, err := readBrokerFrame(c.conn)
	if err != nil {
		return nil, err
	}
	var parsed map[string]any
	if err := json.Unmarshal(resp, &parsed); err != nil {
		return nil, err
	}
	if typ, _ := parsed["type"].(string); typ == "error" {
		message, _ := parsed["message"].(string)
		code, _ := parsed["code"].(string)
		if message == "" {
			message = "broker error"
		}
		if code != "" {
			return parsed, fmt.Errorf("%s: %s", code, message)
		}
		return parsed, fmt.Errorf("%s", message)
	}
	return parsed, nil
}

func strconvQuote(value string) string {
	raw, _ := json.Marshal(value)
	return string(raw)
}

func (c *brokerClient) GetGrokAccessToken(accountID string) (string, error) {
	resp, err := c.call("get_grok_access_token", "", accountID, map[string]any{
		"accountId": accountID,
	})
	if err != nil {
		return "", err
	}
	token, _ := resp["accessToken"].(string)
	if token == "" {
		return "", fmt.Errorf("broker returned empty access token")
	}
	return token, nil
}

func (c *brokerClient) MarkGrokAccount(accountID, status string) {
	if c == nil {
		return
	}
	_, _ = c.call("mark_grok_account", "", accountID+"|"+status, map[string]any{
		"accountId": accountID,
		"status":    status,
	})
}

func (c *brokerClient) ExecuteProvider(providerID, model string, request json.RawMessage) (map[string]any, error) {
	var payload any
	if err := json.Unmarshal(request, &payload); err != nil {
		payload = map[string]any{}
	}
	return c.call("execute_provider", "", providerID+"|"+model, map[string]any{
		"providerId": providerID,
		"modelRoute": model,
		"request":    payload,
	})
}

func (c *brokerClient) Close() {
	if c == nil {
		return
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	c.closed = true
	if c.conn != nil {
		_ = c.conn.Close()
	}
}

var globalBroker struct {
	mu     sync.Mutex
	client *brokerClient
}

func setGlobalBroker(client *brokerClient) {
	globalBroker.mu.Lock()
	defer globalBroker.mu.Unlock()
	if globalBroker.client != nil {
		globalBroker.client.Close()
	}
	globalBroker.client = client
}

func getGlobalBroker() *brokerClient {
	globalBroker.mu.Lock()
	defer globalBroker.mu.Unlock()
	return globalBroker.client
}
