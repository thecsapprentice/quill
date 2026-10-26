-- Administrators' voices on comments (#41). Besides one of their personas,
-- an administrator comments as the shared "Administrators" voice, or under
-- their own staff name. Both are marked as administrators on the board.
-- Login names stay private: the staff name is chosen separately.

-- A public name an administrator chooses for speaking as staff. Unique
-- regardless of case, so one administrator can't pass as another.
ALTER TABLE accounts ADD COLUMN staff_name text;
CREATE UNIQUE INDEX accounts_staff_name_idx ON accounts (lower(staff_name))
    WHERE staff_name IS NOT NULL;

-- persona:        written as `actor_id`.
-- administrators: the shared voice; `account_id` says which one, for the
--                 administrators only.
-- staff:          under `staff_name` as it was when posted.
ALTER TABLE announcement_comments
    ADD COLUMN voice text NOT NULL DEFAULT 'persona'
        CHECK (voice IN ('persona', 'administrators', 'staff')),
    ADD COLUMN account_id uuid REFERENCES accounts (id) ON DELETE SET NULL,
    ADD COLUMN staff_name text,
    ALTER COLUMN actor_id DROP NOT NULL,
    ADD CONSTRAINT announcement_comments_voice_author CHECK (
        (voice = 'persona') = (actor_id IS NOT NULL)
        AND (voice = 'staff') = (staff_name IS NOT NULL)
    );

CREATE INDEX announcement_comments_account_idx ON announcement_comments (account_id)
    WHERE account_id IS NOT NULL;
