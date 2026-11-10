-- Walker placeholder (lane verdict, board #16): the migration runner demands
-- strictly consecutive numbers (database.rs:86-97) and the lanes owning 040-046
-- are not merged onto lane/verdict yet. This no-op only lets the walker reach
-- 047. INTEGRATION: delete this file and register the owning lane's real 0NN.
SELECT 1;
