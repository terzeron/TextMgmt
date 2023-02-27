CREATE TABLE fs_modification
(
    last_modified_time DATETIME NOT NULL DEFAULT NOW()
);
INSERT INTO fs_modification VALUES (NOW());

CREATE TABLE cache_modification
(
    last_modified_time DATETIME NOT NULL DEFAULT NOW()
);
INSERT INTO cache_modification VALUES (NOW());
