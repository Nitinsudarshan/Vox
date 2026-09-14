import React from 'react';

import { accountColor, accountLabel } from '@/lib/calendar';
import type { CalendarAccount } from '@/types/calendar';

interface AccountLegendProps {
  accounts: CalendarAccount[];
}

/**
 * Which colour is which calendar.
 *
 * Only drawn when there is more than one enabled account: with a single
 * calendar every dot is the same colour and a key for it is furniture. This is
 * the half of the colour coding that makes it mean something — a colour
 * without a key is decoration.
 */
export const AccountLegend: React.FC<AccountLegendProps> = ({ accounts }) => {
  const enabled = accounts.filter((account) => account.enabled);
  if (enabled.length < 2) return null;

  const order = enabled.map((account) => account.email);

  return (
    <ul className="flex flex-wrap items-center gap-x-3 gap-y-1">
      {enabled.map((account) => (
        <li key={account.email} className="flex items-center gap-1.5">
          <span
            className={`w-1.5 h-1.5 rounded-full ${accountColor(account.email, order).dot}`}
            aria-hidden
          />
          <span className="text-[11px] text-muted-foreground" title={account.email}>
            {accountLabel(account)}
          </span>
        </li>
      ))}
    </ul>
  );
};
