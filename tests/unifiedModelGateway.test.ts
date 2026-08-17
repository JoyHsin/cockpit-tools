import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type {
  UnifiedGatewayCatalogEntry,
  UnifiedGrokAccountOption,
} from "../src/types/unifiedModelGateway.ts";

function visibleCatalog(models: UnifiedGatewayCatalogEntry[]) {
  return models.filter((model) => model.enabled);
}

describe("unified model catalog contract", () => {
  it("never exposes a disabled external model", () => {
    const models: UnifiedGatewayCatalogEntry[] = [
      {
        id: "gpt-5.4",
        displayName: "GPT-5.4",
        providerId: "official-codex",
        providerType: "official_codex",
        upstreamModel: "gpt-5.4",
        enabled: true,
        conflict: false,
        capabilities: { text: true, streaming: true, tools: true, vision: false, search: false },
        availability: "available",
      },
      {
        id: "grok-4.5",
        displayName: "Grok 4.5 (OAuth)",
        providerId: "grok-oauth",
        providerType: "grok_oauth",
        upstreamModel: "grok-4.5",
        enabled: false,
        conflict: false,
        capabilities: { text: true, streaming: true, tools: true, vision: true, search: true },
        availability: "available",
      },
    ];
    assert.deepEqual(
      visibleCatalog(models).map((model) => model.id),
      ["gpt-5.4"],
    );
  });

  it("keeps OAuth account references selectable without exposing tokens", () => {
    const accounts: UnifiedGrokAccountOption[] = [
      {
        accountId: "acc-1",
        email: "person@example.com",
        authMode: "oauth",
        eligible: true,
        selected: true,
        hasGrokCodeAccess: true,
        source: "cockpit",
      },
      {
        accountId: "acc-key",
        email: "key@example.com",
        authMode: "api_key",
        eligible: false,
        selected: false,
        hasGrokCodeAccess: false,
        source: "cockpit",
        ineligibleReason: "API Key 账号属于 xAI API，不能加入 Grok (OAuth) 池",
      },
    ];
    const oauth = accounts.filter((account) => account.authMode === "oauth");
    assert.equal(oauth.length, 1);
    assert.equal(oauth[0]?.accountId, "acc-1");
    const serialized = JSON.stringify(accounts);
    assert.equal(serialized.includes("refresh"), false);
    assert.equal(serialized.includes("sk-"), false);
  });
});
