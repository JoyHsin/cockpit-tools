import assert from 'node:assert/strict';
import test from 'node:test';

import type { CodexAccount } from '../types/codex.ts';
import {
  buildCodexSubscriptionReminderAcknowledgementKey,
  getCodexSubscriptionExpiryReminders,
} from './codexSubscriptionReminder.ts';

const NOW = Date.UTC(2026, 8, 15, 0, 0, 0);
const DAY = 24 * 60 * 60 * 1000;

function account(partial: Partial<CodexAccount>): CodexAccount {
  return {
    id: 'account-1',
    email: 'account@example.com',
    plan_type: 'plus',
    tokens: { id_token: 'id', access_token: 'access', refresh_token: 'refresh' },
    created_at: NOW,
    last_used: NOW,
    ...partial,
  };
}

test('includes active paid subscriptions with three days or less remaining', () => {
  const reminders = getCodexSubscriptionExpiryReminders([
    account({ subscription_active_until: new Date(NOW + 3 * DAY).toISOString() }),
    account({ id: 'pro', plan_type: 'pro', subscription_active_until: String((NOW + DAY) / 1000) }),
    account({ id: 'team', plan_type: 'chatgpt_team', subscription_active_until: new Date(NOW + 2 * DAY).toISOString() }),
  ], [], NOW);

  assert.deepEqual(reminders.map((item) => item.daysRemaining), [1, 2, 3]);
});

test('excludes expired, distant, free, API key, and unknown-expiry accounts', () => {
  const reminders = getCodexSubscriptionExpiryReminders([
    account({ id: 'expired', subscription_active_until: new Date(NOW).toISOString() }),
    account({ id: 'distant', subscription_active_until: new Date(NOW + 3 * DAY + 1).toISOString() }),
    account({ id: 'free', plan_type: 'free', subscription_active_until: new Date(NOW + DAY).toISOString() }),
    account({ id: 'api', auth_mode: 'apikey', subscription_active_until: new Date(NOW + DAY).toISOString() }),
    account({ id: 'missing', subscription_active_until: undefined }),
  ], [], NOW);

  assert.deepEqual(reminders, []);
});

test('acknowledgement applies only to the same account expiry', () => {
  const firstExpiry = NOW + DAY;
  const acknowledgedKey = buildCodexSubscriptionReminderAcknowledgementKey('account-1', firstExpiry);

  assert.equal(getCodexSubscriptionExpiryReminders([
    account({ subscription_active_until: new Date(firstExpiry).toISOString() }),
  ], [acknowledgedKey], NOW).length, 0);

  assert.equal(getCodexSubscriptionExpiryReminders([
    account({ subscription_active_until: new Date(firstExpiry + DAY).toISOString() }),
  ], [acknowledgedKey], NOW).length, 1);
});
