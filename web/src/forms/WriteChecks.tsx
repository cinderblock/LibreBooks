// Write Checks: the first form, and the one the others follow. Its state
// machine lives in `useTxnForm`; its header, tabs and rules in `PurchaseForm`
// and `purchaseDraft`. This is the window that opens at `/checks/new` and for a
// saved check.

import type { Txn } from '~/api/types';
import { PurchaseForm } from './PurchaseForm';
import { FormLoader } from './useTxnForm';

/** `txnId` edits a saved check; `draftToken` starts from a hand-off; neither: a new one. */
export function WriteChecks({ txnId, draftToken }: { txnId?: string | null; draftToken?: string | null }) {
  return <FormLoader txnId={txnId} draftToken={draftToken} render={(saved: Txn | null, initial: Txn | null) => <PurchaseForm type="Check" saved={saved} initial={initial} />} />;
}
