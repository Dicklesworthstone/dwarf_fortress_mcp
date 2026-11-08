## Recover original Tree and Workshop clock intervals without repeated waits

Tree clock guidance now retires an unattempted interval after its process-local
review is lost, provided the current Control owner retains retirement authority.
Without that authority, a terminal source-loss outcome, or a blocked clock with
no reconcilable interval, it recommends inspection instead of another ineffective
Wait. In-flight intervals retain original-key reconciliation and an independently
authorized safety stop. Old review fields cannot override the actual lifecycle.

Workshop guidance now recognizes the real `intent_recorded` journal state and
offers the same authorized local retirement after a failed preparation guard.
Neither change restores a review, repeats unpause, changes native protocols,
renews an original deadline, or claims Tree supply or Workshop completion.
