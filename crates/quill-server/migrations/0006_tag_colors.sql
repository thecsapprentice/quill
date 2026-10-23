-- Phase 1f: chip colours. A tag type may have a colour pair; a tag may have
-- its own, which replaces its type's. Colours are `#rrggbb`. A NULL text
-- colour means "automatic": black or white, whichever reads better.
--
-- The board turns these into a generated stylesheet, so its CSP needn't
-- allow inline styles. The CHECKs keep anything but a colour out of it.

ALTER TABLE tag_namespaces
    ADD COLUMN chip_bg text CHECK (chip_bg ~ '^#[0-9a-f]{6}$'),
    ADD COLUMN chip_fg text CHECK (chip_fg ~ '^#[0-9a-f]{6}$'),
    ADD CONSTRAINT tag_namespaces_chip_fg_needs_bg CHECK (chip_fg IS NULL OR chip_bg IS NOT NULL);

ALTER TABLE tags
    ADD COLUMN chip_bg text CHECK (chip_bg ~ '^#[0-9a-f]{6}$'),
    ADD COLUMN chip_fg text CHECK (chip_fg ~ '^#[0-9a-f]{6}$'),
    ADD CONSTRAINT tags_chip_fg_needs_bg CHECK (chip_fg IS NULL OR chip_bg IS NOT NULL);

-- Starting colours for the default types, if they still exist. Pale
-- backgrounds with dark text read well on light and dark pages alike.
UPDATE tag_namespaces SET chip_bg = '#dbeafe', chip_fg = '#1e3a8a' WHERE name = 'genre';
UPDATE tag_namespaces SET chip_bg = '#fee2e2', chip_fg = '#7f1d1d' WHERE name = 'content-warning';
UPDATE tag_namespaces SET chip_bg = '#ccfbf1', chip_fg = '#134e4a' WHERE name = 'format';
UPDATE tag_namespaces SET chip_bg = '#fef3c7', chip_fg = '#78350f' WHERE name = 'length';
UPDATE tag_namespaces SET chip_bg = '#ede9fe', chip_fg = '#4c1d95' WHERE name = 'theme';

ALTER TABLE admin_log
    DROP CONSTRAINT admin_log_action_check,
    ADD CONSTRAINT admin_log_action_check CHECK (action IN (
        'report_received', 'discard', 'delete_prompt',
        'create_tag_type', 'update_tag_type',
        'create_tag', 'rename_tag', 'alias_tag', 'delete_tag',
        'approve_tag', 'reject_tag', 'hide_tag', 'show_tag',
        'change_setting', 'recolor_tag'
    ));
