-- Phase 1f: tag colours. A tag type has a colour; a tag may have its own,
-- which overrides its type's. NULL means no colour (or, for a tag, its
-- type's colour). Colours are names from the board's fixed palette, since
-- the board's CSP forbids inline styles.

ALTER TABLE tag_namespaces
    ADD COLUMN color text CHECK (color IN (
        'red', 'orange', 'amber', 'green', 'teal', 'blue', 'indigo', 'purple', 'pink', 'slate'
    ));

ALTER TABLE tags
    ADD COLUMN color text CHECK (color IN (
        'red', 'orange', 'amber', 'green', 'teal', 'blue', 'indigo', 'purple', 'pink', 'slate'
    ));

-- Starting colours for the default types, if they still exist.
UPDATE tag_namespaces SET color = 'blue'   WHERE name = 'genre';
UPDATE tag_namespaces SET color = 'red'    WHERE name = 'content-warning';
UPDATE tag_namespaces SET color = 'teal'   WHERE name = 'format';
UPDATE tag_namespaces SET color = 'amber'  WHERE name = 'length';
UPDATE tag_namespaces SET color = 'purple' WHERE name = 'theme';

ALTER TABLE admin_log
    DROP CONSTRAINT admin_log_action_check,
    ADD CONSTRAINT admin_log_action_check CHECK (action IN (
        'report_received', 'discard', 'delete_prompt',
        'create_tag_type', 'update_tag_type',
        'create_tag', 'rename_tag', 'alias_tag', 'delete_tag',
        'approve_tag', 'reject_tag', 'hide_tag', 'show_tag',
        'change_setting', 'recolor_tag'
    ));
