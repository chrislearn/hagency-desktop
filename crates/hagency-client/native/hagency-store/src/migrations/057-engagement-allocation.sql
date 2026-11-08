-- ADR-186 §A4: the amount the operator granted at approval, or raised by a
-- top-up (§C). NULL means the requested amount (`tokens`) is the allocation,
-- so every engagement decided before this migration keeps its ask as its
-- allocation. `tokens` stays the requester's ask, unchanged. Reservation,
-- draw, side commitment and the Palpo `allocatedTokens` figure all read
-- `COALESCE(allocated_tokens, tokens)`. No board number was assigned; the
-- file takes its list version, 57.
ALTER TABLE engagements ADD COLUMN allocated_tokens INTEGER
    CHECK(allocated_tokens IS NULL OR allocated_tokens>0);
