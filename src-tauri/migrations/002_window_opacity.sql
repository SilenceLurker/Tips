ALTER TABLE item_windows
    ADD COLUMN background_opacity REAL NOT NULL DEFAULT 1
    CHECK (background_opacity >= 0.1 AND background_opacity <= 1);

ALTER TABLE item_windows
    ADD COLUMN text_opacity REAL NOT NULL DEFAULT 1
    CHECK (text_opacity >= 0.2 AND text_opacity <= 1);

UPDATE item_windows
SET background_opacity = opacity;
