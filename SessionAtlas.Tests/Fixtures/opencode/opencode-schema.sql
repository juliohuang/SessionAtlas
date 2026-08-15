CREATE TABLE project (
    id TEXT PRIMARY KEY,
    worktree TEXT NOT NULL,
    name TEXT,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL
);

CREATE TABLE session (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    directory TEXT NOT NULL,
    title TEXT NOT NULL,
    version TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    time_archived INTEGER,
    FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
);
