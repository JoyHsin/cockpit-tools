import {
  isCodexApiKeyAccount,
  isCodexPendingOAuthAccount,
  parseCodexSubscriptionDate,
  type CodexAccount,
} from '../types/codex';

export const CODEX_SUBSCRIPTION_REMINDER_THRESHOLD_DAYS = 3;

const DAY_IN_MS = 24 * 60 * 60 * 1000;
const PAID_PLAN_MARKERS = [
  'plus',
  'pro',
  'team',
  'business',
  'enterprise',
  'edu',
  'go',
] as const;

export interface CodexSubscriptionExpiryReminder {
  accountId: string;
  email: string;
  planLabel: string;
  expiresAtMs: number;
  daysRemaining: number;
  acknowledgementKey: string;
}

function isPaidCodexSubscription(account: CodexAccount): boolean {
  if (isCodexApiKeyAccount(account) || isCodexPendingOAuthAccount(account)) {
    return false;
  }

  const plan = (account.plan_type || '').trim().toLowerCase();
  return PAID_PLAN_MARKERS.some((marker) => plan.includes(marker));
}

export function buildCodexSubscriptionReminderAcknowledgementKey(
  accountId: string,
  expiresAtMs: number,
): string {
  return `${encodeURIComponent(accountId)}:${expiresAtMs}`;
}

export function getCodexSubscriptionExpiryReminders(
  accounts: CodexAccount[],
  acknowledgedKeys: Iterable<string>,
  nowMs = Date.now(),
  thresholdDays = CODEX_SUBSCRIPTION_REMINDER_THRESHOLD_DAYS,
): CodexSubscriptionExpiryReminder[] {
  const acknowledged = new Set(acknowledgedKeys);
  const thresholdMs = Math.max(0, thresholdDays) * DAY_IN_MS;

  return accounts
    .flatMap((account): CodexSubscriptionExpiryReminder[] => {
      if (!isPaidCodexSubscription(account)) return [];

      const expiresAt = parseCodexSubscriptionDate(account.subscription_active_until);
      if (!expiresAt) return [];

      const expiresAtMs = expiresAt.getTime();
      const remainingMs = expiresAtMs - nowMs;
      if (remainingMs <= 0 || remainingMs > thresholdMs) return [];

      const acknowledgementKey = buildCodexSubscriptionReminderAcknowledgementKey(
        account.id,
        expiresAtMs,
      );
      if (acknowledged.has(acknowledgementKey)) return [];

      return [{
        accountId: account.id,
        email: account.email?.trim() || account.account_name?.trim() || account.id,
        planLabel: (account.plan_type || '').trim().toUpperCase(),
        expiresAtMs,
        daysRemaining: Math.max(1, Math.ceil(remainingMs / DAY_IN_MS)),
        acknowledgementKey,
      }];
    })
    .sort((left, right) =>
      left.expiresAtMs - right.expiresAtMs || left.email.localeCompare(right.email),
    );
}
