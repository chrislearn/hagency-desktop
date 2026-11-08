-- Task #61 row 6: bridge-matrix.js:3374-3385 — an incidental answer to a
-- message with no source thread starts a NEW thread rooted at that message.
-- The flag is submitted with the reply and read at send time; it does not
-- change which rows are current, so current_final_replies is untouched.
ALTER TABLE final_replies ADD COLUMN incidental INTEGER NOT NULL DEFAULT 0 CHECK(incidental IN (0,1));
