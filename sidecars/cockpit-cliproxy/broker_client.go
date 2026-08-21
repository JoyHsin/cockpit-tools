package main

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
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
	socketPath string
	secrets    *brokerSecrets
	conn       net.Conn
	key        []byte
	seq        uint64
	mu         sync.Mutex
	closed     bool
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

func signBroker(sessionKey, payload []byte) string {
	mac := hmac.New(sha256.New, sessionKey)
	mac.Write(payload)
	return hex.EncodeToString(mac.Sum(nil))
}

func writeBrokerFrame(w io.Writer, payload []byte) error {
	if len(payload) == 0 || len(payload) > brokerMaxFrame {
		return fmt.Errorf("invalid broker frame length %d", len(payload))
	}
	var length [4]byte
	binary.LittleEndian.PutUint32(length[:], uint32(len(payload)))
	if _, err := w.Write(length[:]); err != nil {
		return err
	}
	_, err := w.Write(payload)
	return err
}

func readBrokerFrame(r io.Reader) ([]byte, error) {
	var lengthBuf [4]byte
	if _, err := io.ReadFull(r, lengthBuf[:]); err != nil {
		return nil, err
	}
	length := int(binary.LittleEndian.Uint32(lengthBuf[:]))
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
	client := &brokerClient{
		socketPath: socketPath,
		secrets:    secrets,
		key:        secrets.sessionKey,
		seq:        1,
	}
	if err := client.reconnectLocked(); err != nil {
		return nil, err
	}
	return client, nil
}

func (c *brokerClient) reconnectLocked() error {
	if c.conn != nil {
		_ = c.conn.Close()
		c.conn = nil
	}
	var conn net.Conn
	var err error
	for attempt := 0; attempt < 20; attempt++ {
		conn, err = dialBroker(c.socketPath)
		if err == nil {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	if err != nil {
		return fmt.Errorf("dial broker socket: %w", err)
	}
	nonceHex := hex.EncodeToString(c.secrets.nonce)
	pid := os.Getpid()
	payload := []byte(fmt.Sprintf("hello|%d|%d|%s", brokerProtocol, pid, nonceHex))
	hello := map[string]any{
		"protocolVersion": brokerProtocol,
		"childPid":        pid,
		"nonce":           nonceHex,
		"hmac":            signBroker(c.secrets.sessionKey, payload),
	}
	raw, err := json.Marshal(hello)
	if err != nil {
		_ = conn.Close()
		return err
	}
	if err := writeBrokerFrame(conn, raw); err != nil {
		_ = conn.Close()
		return err
	}
	response, err := readBrokerFrame(conn)
	if err != nil {
		_ = conn.Close()
		return err
	}
	var parsed struct {
		Type string `json:"type"`
	}
	if err := json.Unmarshal(response, &parsed); err != nil || parsed.Type != "hello_ok" {
		_ = conn.Close()
		return fmt.Errorf("broker handshake rejected")
	}
	c.conn = conn
	c.seq = 1
	c.closed = false
	return nil
}

func (c *brokerClient) sendAndReceiveLocked(kind, bodyKey, body string, extra map[string]any) (map[string]any, error) {
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

func (c *brokerClient) call(kind, bodyKey, body string, extra map[string]any) (map[string]any, error) {
	if c == nil {
		return nil, fmt.Errorf("broker is not connected")
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed || c.conn == nil {
		if err := c.reconnectLocked(); err != nil {
			return nil, fmt.Errorf("broker reconnect: %w", err)
		}
	}
	resp, err := c.sendAndReceiveLocked(kind, bodyKey, body, extra)
	if err != nil {
		// 遇到断连错误时，自动尝试重新握手重试一次
		if recErr := c.reconnectLocked(); recErr == nil {
			resp, err = c.sendAndReceiveLocked(kind, bodyKey, body, extra)
		}
	}
	return resp, err
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

func (c *brokerClient) ExecuteProviderStream(providerID, model string, request json.RawMessage, onChunk func([]byte) error) (map[string]any, error) {
	if c == nil {
		return nil, fmt.Errorf("broker is not connected")
	}
	if onChunk == nil {
		return nil, fmt.Errorf("provider stream callback is required")
	}
	var payload any
	if err := json.Unmarshal(request, &payload); err != nil {
		payload = map[string]any{}
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return nil, fmt.Errorf("broker connection closed")
	}
	seq := c.seq
	c.seq++
	body := providerID + "|" + model
	req := map[string]any{
		"type":       "execute_provider_request",
		"seq":        seq,
		"providerId": providerID,
		"modelRoute": model,
		"request":    payload,
		"hmac":       signBroker(c.key, []byte(fmt.Sprintf("%d|execute_provider|%s", seq, body))),
	}
	raw, err := json.Marshal(req)
	if err != nil {
		return nil, err
	}
	if err := writeBrokerFrame(c.conn, raw); err != nil {
		return nil, err
	}
	meta := map[string]any{"type": "provider_response"}
	for {
		frame, err := readBrokerFrame(c.conn)
		if err != nil {
			return nil, err
		}
		var parsed map[string]any
		if err := json.Unmarshal(frame, &parsed); err != nil {
			return nil, err
		}
		typ, _ := parsed["type"].(string)
		if typ == "error" {
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
		if typ == "provider_stream_chunk" {
			data, _ := parsed["data"].(string)
			chunk, err := base64.StdEncoding.DecodeString(data)
			if err != nil {
				return nil, fmt.Errorf("invalid provider stream chunk: %w", err)
			}
			if err := onChunk(chunk); err != nil {
				return nil, err
			}
			meta["status"] = parsed["status"]
			meta["contentType"] = parsed["contentType"]
			continue
		}
		if typ == "provider_stream_done" {
			meta["status"] = parsed["status"]
			meta["contentType"] = parsed["contentType"]
			return meta, nil
		}
		if typ == "provider_response" {
			return parsed, nil
		}
	}
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
