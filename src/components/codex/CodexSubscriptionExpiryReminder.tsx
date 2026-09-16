import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import {
  hydrateUserMemory,
  persistUserMemoryList,
  readUserMemoryList,
  USER_MEMORY_LISTS,
} from '../../utils/userMemory';
import {
  getCodexSubscriptionExpiryReminders,
  type CodexSubscriptionExpiryReminder,
} from '../../utils/codexSubscriptionReminder';
import './CodexSubscriptionExpiryReminder.css';

export function CodexSubscriptionExpiryReminderHost() {
  const { t, i18n } = useTranslation();
  const [reminders, setReminders] = useState<CodexSubscriptionExpiryReminder[]>([]);

  useEffect(() => {
    let disposed = false;

    void (async () => {
      await hydrateUserMemory();
      await useCodexAccountStore.getState().fetchAccounts();
      if (disposed) return;

      const acknowledged = readUserMemoryList(USER_MEMORY_LISTS.codexSubscriptionExpiry);
      const accounts = useCodexAccountStore.getState().accounts;
      setReminders(getCodexSubscriptionExpiryReminders(accounts, acknowledged));
    })();

    return () => {
      disposed = true;
    };
  }, []);

  if (reminders.length === 0) return null;

  const handleRecharged = () => {
    const acknowledged = readUserMemoryList(USER_MEMORY_LISTS.codexSubscriptionExpiry);
    persistUserMemoryList(USER_MEMORY_LISTS.codexSubscriptionExpiry, [
      ...acknowledged,
      ...reminders.map((item) => item.acknowledgementKey),
    ]);
    setReminders([]);
  };

  const dateFormatter = new Intl.DateTimeFormat(i18n.resolvedLanguage || i18n.language, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });

  return (
    <div className="modal-overlay codex-subscription-reminder-overlay" role="presentation">
      <section
        className="modal codex-subscription-reminder-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-subscription-reminder-title"
      >
        <div className="modal-header">
          <h2 id="codex-subscription-reminder-title">
            {t('codex.subscriptionReminder.title', 'Codex 订阅即将到期')}
          </h2>
        </div>

        <div className="codex-subscription-reminder-body">
          <p className="codex-subscription-reminder-description">
            {t(
              'codex.subscriptionReminder.description',
              '以下 Codex 账号的订阅有效期不超过 3 天，请及时充值续费。',
            )}
          </p>
          <ul className="codex-subscription-reminder-list">
            {reminders.map((reminder) => (
              <li key={reminder.acknowledgementKey} className="codex-subscription-reminder-item">
                <div className="codex-subscription-reminder-account">
                  <strong title={reminder.email}>{reminder.email}</strong>
                  <span className="codex-subscription-reminder-plan">{reminder.planLabel}</span>
                  <div className="codex-subscription-reminder-date">
                    {t('codex.subscriptionReminder.expiresAt', '到期时间：{{date}}', {
                      date: dateFormatter.format(reminder.expiresAtMs),
                    })}
                  </div>
                </div>
                <span className="codex-subscription-reminder-days">
                  {t('codex.subscriptionReminder.daysRemaining', '还剩 {{count}} 天', {
                    count: reminder.daysRemaining,
                  })}
                </span>
              </li>
            ))}
          </ul>
        </div>

        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={() => setReminders([])}>
            {t('codex.subscriptionReminder.confirm', '确认')}
          </button>
          <button type="button" className="btn btn-primary" onClick={handleRecharged}>
            {t('codex.subscriptionReminder.recharged', '我已充值，不再提示')}
          </button>
        </div>
      </section>
    </div>
  );
}
