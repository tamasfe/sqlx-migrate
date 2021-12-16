-- Migration SQL for initial_migration

CREATE TABLE IF NOT EXISTS users (
    user_id SERIAL PRIMARY KEY,
    username varchar(25) NOT NULL,
    owns_plush_sharks BOOLEAN NOT NULL
);

-- An example user:
INSERT INTO users
    (username, owns_plush_sharks)
VALUES
    ('tom', TRUE);
