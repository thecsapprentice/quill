-- Frequent tags (#55): the post form suggests the tags a writer uses most,
-- from the persona they're posting as unless they choose all of them.

ALTER TABLE accounts ADD COLUMN suggest_all_personas boolean NOT NULL DEFAULT false;
