ALTER TABLE users ADD COLUMN phone_number VARCHAR(32);

ALTER TABLE predictions
    DROP CONSTRAINT predictions_user_id_fkey,
    ADD CONSTRAINT predictions_user_id_fkey
        FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE special_predictions
    DROP CONSTRAINT special_predictions_user_id_fkey,
    ADD CONSTRAINT special_predictions_user_id_fkey
        FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
