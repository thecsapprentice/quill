-- Tag type limits (#52): how many tags of each type a prompt may carry,
-- replacing the yes/no `required`. A required type becomes "at least one";
-- no type has a maximum until an administrator sets one.

ALTER TABLE tag_namespaces
    ADD COLUMN min_tags integer NOT NULL DEFAULT 0,
    ADD COLUMN max_tags integer;

UPDATE tag_namespaces SET min_tags = 1 WHERE required;

ALTER TABLE tag_namespaces
    DROP COLUMN required,
    ADD CONSTRAINT tag_namespaces_limits_check CHECK (
        min_tags >= 0 AND (max_tags IS NULL OR (max_tags >= 1 AND max_tags >= min_tags))
    );
