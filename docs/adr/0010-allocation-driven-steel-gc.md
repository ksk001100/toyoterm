# 0010: Use Steel's allocation-driven collector

- Status: Accepted
- Date: 2026-10-31
- Refines the collection policy of ADR 0008; ownership and rollback are unchanged.

## Context

Forcing Steel 0.8.3 full collection after every successful or failed request
and configuration load repeatedly expands its free list before compacting it.
A plain Engine reproduces a 22 to 721 MiB allocation cycle with no retained
application results. Active personal widgets drive ordinary GUI working set
above 1 GiB. See the October 31 memory investigation.

## Decision

Use Steel's normal allocation-driven collector. Do not request full collection
on host request completion, rollback or configuration loading. Increasing the
interval of forced collection would merely delay the same expansion cycle.

Collection and all VM access remain on the single script owner thread. Keep
transaction checkpoints and temporary rollback values rooted in VM globals
while executing Scheme, because allocation-driven or user-requested collection
can run inside a callback. Keep explicit full-collection regression tests and
diagnostics separate from ordinary production memory measurements.

Avoid re-encoding unchanged config, theme and registration snapshots on every
callback. Store their revision in the persistent application map, invalidate it
on snapshot-visible mutations, and update the native cache only after complete
validation. Rollback and fresh-VM reload preserve revision/state consistency.

## Consequences

Collection follows actual allocation pressure instead of callback frequency.
The host makes no immediate reclamation or RSS reduction guarantee after a
request. Native outputs, command order, transactional settings, registrations,
task/selector cleanup and atomic fresh-VM reload behavior remain unchanged.
The VM is still trusted local code; user Scheme can explicitly force collection.
Memory validation must include automatic collection, failed callbacks, retained
closures and repeated reloads, with OS resident and committed memory separated.
