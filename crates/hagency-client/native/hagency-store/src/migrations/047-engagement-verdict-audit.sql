-- Console verdict audit (board #16, parity lib/engagement-store.js:299-321
-- `record` + :804-806 `listAudit`): the decisions table gains the audit's own
-- columns — the retained entry shape is {type, at, ...detail}; native records
-- only the decision-own facts, everything the console list needs.
ALTER TABLE decisions ADD COLUMN kind TEXT;
ALTER TABLE decisions ADD COLUMN at INTEGER;
