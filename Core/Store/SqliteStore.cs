using System.Data;
using System.Globalization;
using System.Text;
using Microsoft.Data.Sqlite;
using SessionAtlas.Core.Scanner;
using SessionAtlas.Models;

namespace SessionAtlas.Core.Store;

/// <summary>
/// SQLite 本地存储 - 项目索引与会话记录
/// </summary>
public class SqliteStore : IDisposable
{
    private readonly SqliteConnection _connection;
    private readonly string _dbPath;

    public SqliteStore(string? databasePath = null)
    {
        if (string.IsNullOrWhiteSpace(databasePath))
        {
            var home = ScannerRegistry.GetHomeDirectory();
            var appDir = Path.Combine(home, ".sessionatlas");
            Directory.CreateDirectory(appDir);
            _dbPath = Path.Combine(appDir, "index.db");
        }
        else
        {
            _dbPath = Path.GetFullPath(databasePath);
            var parent = Path.GetDirectoryName(_dbPath);
            if (!string.IsNullOrEmpty(parent))
                Directory.CreateDirectory(parent);
        }

        var connectionString = new SqliteConnectionStringBuilder
        {
            DataSource = _dbPath,
            // Explicit paths are primarily used by isolated tests and
            // short-lived tools; disabling pooling lets their temp
            // directories be removed immediately after Dispose().
            Pooling = string.IsNullOrWhiteSpace(databasePath)
        }.ToString();
        _connection = new SqliteConnection(connectionString);
        _connection.Open();
        using (var pragma = _connection.CreateCommand())
        {
            pragma.CommandText = "PRAGMA foreign_keys = ON";
            pragma.ExecuteNonQuery();
        }
        InitializeSchema();
    }

    private void InitializeSchema()
    {
        using var cmd = _connection.CreateCommand();
        cmd.CommandText = @"
            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                last_accessed_at TEXT,
                first_seen_at TEXT,
                git_branch TEXT,
                git_remote_url TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_projects_path ON projects(path);
            CREATE INDEX IF NOT EXISTS idx_projects_last_accessed ON projects(last_accessed_at);

            CREATE VIRTUAL TABLE IF NOT EXISTS projects_fts USING fts5(name, path);

            CREATE TABLE IF NOT EXISTS tool_usages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                tool_key TEXT NOT NULL,
                last_used_at TEXT,
                session_count INTEGER DEFAULT 1,
                last_session_id TEXT,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_usages_project ON tool_usages(project_id);
            CREATE INDEX IF NOT EXISTS idx_usages_tool ON tool_usages(tool_key);

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                project_path TEXT NOT NULL,
                tool_key TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                started_at TEXT,
                ended_at TEXT,
                session_id_from_tool TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);
        ";
        cmd.ExecuteNonQuery();
        MigrateToolUsageIdentity();
        EnsureNativePathIndex();
        EnsureToolKeyIndex();
        RebuildSearchIndex();
    }

    private void EnsureNativePathIndex()
    {
        if (!OperatingSystem.IsWindows())
            return;
        using var command = _connection.CreateCommand();
        command.CommandText = "CREATE INDEX IF NOT EXISTS idx_projects_path_nocase ON projects(path COLLATE NOCASE)";
        command.ExecuteNonQuery();
    }

    private void EnsureToolKeyIndex()
    {
        using var command = _connection.CreateCommand();
        command.CommandText = @"
            CREATE INDEX IF NOT EXISTS idx_usages_tool_nocase
            ON tool_usages(tool_key COLLATE NOCASE, project_id)
        ";
        command.ExecuteNonQuery();
    }

    /// <summary>
    /// Older releases appended a tool usage on every scan because the table had
    /// no uniqueness constraint. Collapse those rows before enforcing the
    /// snapshot identity of one row per (project, tool).
    /// </summary>
    private void MigrateToolUsageIdentity()
    {
        using var tx = _connection.BeginTransaction();
        try
        {
            using var migrate = _connection.CreateCommand();
            migrate.Transaction = tx;
            migrate.CommandText = @"
                DELETE FROM tool_usages
                WHERE NOT EXISTS (
                    SELECT 1 FROM projects p WHERE p.id = tool_usages.project_id
                );

                WITH ranked AS (
                    SELECT
                        id,
                        ROW_NUMBER() OVER (
                            PARTITION BY project_id, tool_key COLLATE NOCASE
                            ORDER BY last_used_at DESC, id DESC
                        ) AS rank,
                        MAX(session_count) OVER (
                            PARTITION BY project_id, tool_key COLLATE NOCASE
                        ) AS max_session_count
                    FROM tool_usages
                )
                UPDATE tool_usages
                SET session_count = (
                    SELECT max_session_count
                    FROM ranked
                    WHERE ranked.id = tool_usages.id
                )
                WHERE id IN (SELECT id FROM ranked WHERE rank = 1);

                WITH ranked AS (
                    SELECT
                        id,
                        ROW_NUMBER() OVER (
                            PARTITION BY project_id, tool_key COLLATE NOCASE
                            ORDER BY last_used_at DESC, id DESC
                        ) AS rank
                    FROM tool_usages
                )
                DELETE FROM tool_usages
                WHERE id IN (SELECT id FROM ranked WHERE rank > 1);

                CREATE UNIQUE INDEX IF NOT EXISTS idx_usages_project_tool
                ON tool_usages(project_id, tool_key COLLATE NOCASE);
            ";
            migrate.ExecuteNonQuery();
            tx.Commit();
        }
        catch
        {
            tx.Rollback();
            throw;
        }
    }

    public void UpsertProject(Project project)
    {
        var normalizedPath = ProjectPathSemantics.NormalizeNative(project.Path);
        var pathComparison = OperatingSystem.IsWindows()
            ? "path = @path COLLATE NOCASE"
            : "path = @path";
        using var tx = _connection.BeginTransaction();
        try
        {
            string actualId = project.Id;
            long rowid = 0;
            using var selectCmd = _connection.CreateCommand();
            selectCmd.Transaction = tx;
            selectCmd.CommandText = $"SELECT id, rowid FROM projects WHERE {pathComparison} LIMIT 1";
            selectCmd.Parameters.AddWithValue("@path", normalizedPath);
            using (var reader = selectCmd.ExecuteReader())
            {
                if (reader.Read())
                {
                    actualId = reader.GetString(0);
                    rowid = reader.GetInt64(1);
                }
            }

            using var cmd = _connection.CreateCommand();
            cmd.Transaction = tx;
            cmd.CommandText = rowid == 0
                ? @"
                    INSERT INTO projects (id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url)
                    VALUES (@id, @path, @last, @first, @branch, @remote)
                  "
                : @"
                    UPDATE projects SET
                        last_accessed_at = @last,
                        git_branch = COALESCE(@branch, git_branch),
                        git_remote_url = COALESCE(@remote, git_remote_url)
                    WHERE id = @id
                  ";
            cmd.Parameters.AddWithValue("@id", actualId);
            cmd.Parameters.AddWithValue("@path", normalizedPath);
            cmd.Parameters.AddWithValue("@last", AsUtc(project.LastAccessedAt).ToString("O"));
            cmd.Parameters.AddWithValue("@first", AsUtc(project.FirstSeenAt).ToString("O"));
            cmd.Parameters.AddWithValue("@branch", project.GitBranch ?? (object)DBNull.Value);
            cmd.Parameters.AddWithValue("@remote", project.GitRemoteUrl ?? (object)DBNull.Value);
            cmd.ExecuteNonQuery();

            if (rowid == 0)
            {
                using var rowIdCommand = _connection.CreateCommand();
                rowIdCommand.Transaction = tx;
                rowIdCommand.CommandText = "SELECT rowid FROM projects WHERE id = @id";
                rowIdCommand.Parameters.AddWithValue("@id", actualId);
                rowid = (long)(rowIdCommand.ExecuteScalar()
                    ?? throw new DataException("Inserted project row was not found."));
            }

            // 同步 FTS5 外部内容表：先删旧行再插新行（name 取目录名）
            var projectName = ProjectPathSemantics.GetDisplayName(normalizedPath);
            using var ftsDel = _connection.CreateCommand();
            ftsDel.Transaction = tx;
            ftsDel.CommandText = "DELETE FROM projects_fts WHERE rowid = @rid";
            ftsDel.Parameters.AddWithValue("@rid", rowid);
            ftsDel.ExecuteNonQuery();
            using var ftsIns = _connection.CreateCommand();
            ftsIns.Transaction = tx;
            ftsIns.CommandText = "INSERT INTO projects_fts (rowid, name, path) VALUES (@rid, @name, @path)";
            ftsIns.Parameters.AddWithValue("@rid", rowid);
            ftsIns.Parameters.AddWithValue("@name", projectName);
            ftsIns.Parameters.AddWithValue("@path", normalizedPath);
            ftsIns.ExecuteNonQuery();

            // 更新 tool_usages
            foreach (var usage in project.ToolUsages)
            {
                using var usageCmd = _connection.CreateCommand();
                usageCmd.Transaction = tx;
                usageCmd.CommandText = @"
                    INSERT INTO tool_usages (project_id, tool_name, tool_key, last_used_at, session_count, last_session_id)
                    VALUES (@pid, @tname, @tkey, @tlast, @tcount, @tsid)
                    ON CONFLICT DO UPDATE SET
                        tool_name = excluded.tool_name,
                        last_used_at = excluded.last_used_at,
                        session_count = excluded.session_count,
                        last_session_id = excluded.last_session_id
                ";
                usageCmd.Parameters.AddWithValue("@pid", actualId);
                usageCmd.Parameters.AddWithValue("@tname", usage.ToolName);
                usageCmd.Parameters.AddWithValue("@tkey", usage.ToolKey);
                usageCmd.Parameters.AddWithValue("@tlast", usage.LastUsedAt.ToString("O"));
                usageCmd.Parameters.AddWithValue("@tcount", usage.SessionCount);
                usageCmd.Parameters.AddWithValue("@tsid", usage.LastSessionId ?? (object)DBNull.Value);
                usageCmd.ExecuteNonQuery();
            }

            tx.Commit();
        }
        catch
        {
            tx.Rollback();
            throw;
        }
    }

    /// <summary>
    /// Atomically replace the snapshots for the declared successfully scanned
    /// tools. Tools omitted from <paramref name="scannedToolKeys"/> are
    /// preserved, while a declared tool with no incoming rows is cleared.
    /// </summary>
    public void ReplaceToolSnapshots(
        IReadOnlyCollection<Project> projects,
        IReadOnlyCollection<string> scannedToolKeys)
    {
        ArgumentNullException.ThrowIfNull(projects);
        ArgumentNullException.ThrowIfNull(scannedToolKeys);

        var toolKeys = ValidateSnapshot(projects, scannedToolKeys);
        if (toolKeys.Count == 0)
            throw new ArgumentException(
                "At least one successfully scanned tool key is required.",
                nameof(scannedToolKeys));

        using var tx = _connection.BeginTransaction();
        try
        {
            PrepareSnapshotTables(tx);
            foreach (var toolKey in toolKeys)
                InsertScannedTool(tx, toolKey);

            foreach (var project in projects)
            {
                var actualProjectId = UpsertSnapshotProject(tx, project);
                foreach (var usage in project.ToolUsages)
                {
                    UpsertSnapshotUsage(tx, actualProjectId, usage);
                    InsertSnapshotUsageIdentity(tx, actualProjectId, usage.ToolKey);
                }
            }

            DeleteStaleSnapshotRows(tx);
            RecomputeProjectActivity(tx);
            RebuildFts(tx);
            tx.Commit();
        }
        catch
        {
            tx.Rollback();
            throw;
        }
    }

    private static HashSet<string> ValidateSnapshot(
        IReadOnlyCollection<Project> projects,
        IReadOnlyCollection<string> scannedToolKeys)
    {
        var toolKeys = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var toolKey in scannedToolKeys)
        {
            if (string.IsNullOrWhiteSpace(toolKey))
                throw new ArgumentException("Scanned tool keys cannot be empty.", nameof(scannedToolKeys));
            toolKeys.Add(toolKey.Trim());
        }

        var paths = new HashSet<string>(ProjectPathSemantics.NativeComparer);

        foreach (var project in projects)
        {
            if (string.IsNullOrWhiteSpace(project.Path))
                throw new ArgumentException("Snapshot project paths cannot be empty.", nameof(projects));

            var fullPath = ProjectPathSemantics.NormalizeNative(project.Path);
            if (!paths.Add(fullPath))
                throw new ArgumentException($"Snapshot contains duplicate project path: {fullPath}", nameof(projects));
            if (project.ToolUsages.Count == 0)
                throw new ArgumentException(
                    $"Snapshot project has no tool usages: {fullPath}",
                    nameof(projects));

            var projectToolKeys = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var usage in project.ToolUsages)
            {
                if (!toolKeys.Contains(usage.ToolKey))
                    throw new ArgumentException(
                        $"Usage tool '{usage.ToolKey}' is not declared as successfully scanned.",
                        nameof(projects));
                if (!projectToolKeys.Add(usage.ToolKey))
                    throw new ArgumentException(
                        $"Project contains duplicate usage for tool '{usage.ToolKey}': {fullPath}",
                        nameof(projects));
                if (usage.SessionCount < 0)
                    throw new ArgumentException(
                        $"Session count cannot be negative for tool '{usage.ToolKey}': {fullPath}",
                        nameof(projects));
            }
        }

        return toolKeys;
    }

    private static DateTime AsUtc(DateTime value) =>
        value.Kind switch
        {
            DateTimeKind.Utc => value,
            DateTimeKind.Local => value.ToUniversalTime(),
            _ => DateTime.SpecifyKind(value, DateTimeKind.Utc)
        };

    private void PrepareSnapshotTables(SqliteTransaction tx)
    {
        using var command = _connection.CreateCommand();
        command.Transaction = tx;
        command.CommandText = @"
            CREATE TEMP TABLE IF NOT EXISTS scanned_tool_keys (
                tool_key TEXT PRIMARY KEY COLLATE NOCASE
            ) WITHOUT ROWID;
            CREATE TEMP TABLE IF NOT EXISTS snapshot_tool_usages (
                project_id TEXT NOT NULL,
                tool_key TEXT NOT NULL COLLATE NOCASE,
                PRIMARY KEY (project_id, tool_key)
            ) WITHOUT ROWID;
            DELETE FROM scanned_tool_keys;
            DELETE FROM snapshot_tool_usages;
        ";
        command.ExecuteNonQuery();
    }

    private void InsertScannedTool(SqliteTransaction tx, string toolKey)
    {
        using var command = _connection.CreateCommand();
        command.Transaction = tx;
        command.CommandText = "INSERT INTO scanned_tool_keys (tool_key) VALUES (@toolKey)";
        command.Parameters.AddWithValue("@toolKey", toolKey);
        command.ExecuteNonQuery();
    }

    private string UpsertSnapshotProject(SqliteTransaction tx, Project project)
    {
        var normalizedPath = ProjectPathSemantics.NormalizeNative(project.Path);
        var pathComparison = OperatingSystem.IsWindows()
            ? "path = @path COLLATE NOCASE"
            : "path = @path";

        using var find = _connection.CreateCommand();
        find.Transaction = tx;
        find.CommandText = $"SELECT id FROM projects WHERE {pathComparison} LIMIT 1";
        find.Parameters.AddWithValue("@path", normalizedPath);
        var existingId = find.ExecuteScalar() as string;

        if (existingId is not null)
        {
            using var update = _connection.CreateCommand();
            update.Transaction = tx;
            update.CommandText = @"
                UPDATE projects
                SET
                    git_branch = COALESCE(@branch, git_branch),
                    git_remote_url = COALESCE(@remote, git_remote_url)
                WHERE id = @id
            ";
            update.Parameters.AddWithValue("@id", existingId);
            update.Parameters.AddWithValue("@branch", project.GitBranch ?? (object)DBNull.Value);
            update.Parameters.AddWithValue("@remote", project.GitRemoteUrl ?? (object)DBNull.Value);
            update.ExecuteNonQuery();
            return existingId;
        }

        var projectId = string.IsNullOrWhiteSpace(project.Id)
            ? Guid.NewGuid().ToString("N")
            : project.Id;
        using var insert = _connection.CreateCommand();
        insert.Transaction = tx;
        insert.CommandText = @"
            INSERT INTO projects
                (id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url)
            VALUES
                (@id, @path, @last, @first, @branch, @remote)
        ";
        insert.Parameters.AddWithValue("@id", projectId);
        insert.Parameters.AddWithValue("@path", normalizedPath);
        insert.Parameters.AddWithValue("@last", AsUtc(project.LastAccessedAt).ToString("O"));
        insert.Parameters.AddWithValue("@first", AsUtc(project.FirstSeenAt).ToString("O"));
        insert.Parameters.AddWithValue("@branch", project.GitBranch ?? (object)DBNull.Value);
        insert.Parameters.AddWithValue("@remote", project.GitRemoteUrl ?? (object)DBNull.Value);
        insert.ExecuteNonQuery();
        return projectId;
    }

    private void UpsertSnapshotUsage(
        SqliteTransaction tx,
        string projectId,
        ToolUsage usage)
    {
        using var command = _connection.CreateCommand();
        command.Transaction = tx;
        command.CommandText = @"
            INSERT INTO tool_usages
                (project_id, tool_name, tool_key, last_used_at, session_count, last_session_id)
            VALUES
                (@projectId, @toolName, @toolKey, @lastUsed, @sessionCount, @lastSessionId)
            ON CONFLICT DO UPDATE SET
                tool_name = excluded.tool_name,
                tool_key = excluded.tool_key,
                last_used_at = excluded.last_used_at,
                session_count = excluded.session_count,
                last_session_id = excluded.last_session_id
        ";
        command.Parameters.AddWithValue("@projectId", projectId);
        command.Parameters.AddWithValue("@toolName", usage.ToolName);
        command.Parameters.AddWithValue("@toolKey", usage.ToolKey);
        command.Parameters.AddWithValue("@lastUsed", AsUtc(usage.LastUsedAt).ToString("O"));
        command.Parameters.AddWithValue("@sessionCount", usage.SessionCount);
        command.Parameters.AddWithValue("@lastSessionId", usage.LastSessionId ?? (object)DBNull.Value);
        command.ExecuteNonQuery();
    }

    private void InsertSnapshotUsageIdentity(
        SqliteTransaction tx,
        string projectId,
        string toolKey)
    {
        using var command = _connection.CreateCommand();
        command.Transaction = tx;
        command.CommandText = @"
            INSERT INTO snapshot_tool_usages (project_id, tool_key)
            VALUES (@projectId, @toolKey)
        ";
        command.Parameters.AddWithValue("@projectId", projectId);
        command.Parameters.AddWithValue("@toolKey", toolKey);
        command.ExecuteNonQuery();
    }

    private void DeleteStaleSnapshotRows(SqliteTransaction tx)
    {
        using var command = _connection.CreateCommand();
        command.Transaction = tx;
        command.CommandText = @"
            DELETE FROM tool_usages
            WHERE EXISTS (
                SELECT 1
                FROM scanned_tool_keys scanned
                WHERE scanned.tool_key = tool_usages.tool_key COLLATE NOCASE
            )
            AND NOT EXISTS (
                SELECT 1
                FROM snapshot_tool_usages snapshot
                WHERE snapshot.project_id = tool_usages.project_id
                  AND snapshot.tool_key = tool_usages.tool_key COLLATE NOCASE
            );

            DELETE FROM projects
            WHERE NOT EXISTS (
                SELECT 1 FROM tool_usages usage WHERE usage.project_id = projects.id
            );
        ";
        command.ExecuteNonQuery();
    }

    private void RecomputeProjectActivity(SqliteTransaction tx)
    {
        using var command = _connection.CreateCommand();
        command.Transaction = tx;
        command.CommandText = @"
            UPDATE projects
            SET last_accessed_at = (
                SELECT MAX(usage.last_used_at)
                FROM tool_usages usage
                WHERE usage.project_id = projects.id
            )
        ";
        command.ExecuteNonQuery();
    }

    private void RebuildFts(SqliteTransaction tx)
    {
        using (var delete = _connection.CreateCommand())
        {
            delete.Transaction = tx;
            delete.CommandText = "DELETE FROM projects_fts";
            delete.ExecuteNonQuery();
        }

        var rows = new List<(long RowId, string Path)>();
        using (var select = _connection.CreateCommand())
        {
            select.Transaction = tx;
            select.CommandText = "SELECT rowid, path FROM projects ORDER BY rowid";
            using var reader = select.ExecuteReader();
            while (reader.Read())
                rows.Add((reader.GetInt64(0), reader.GetString(1)));
        }

        foreach (var row in rows)
        {
            var name = ProjectPathSemantics.GetDisplayName(row.Path);
            using var insert = _connection.CreateCommand();
            insert.Transaction = tx;
            insert.CommandText = @"
                INSERT INTO projects_fts (rowid, name, path)
                VALUES (@rowId, @name, @path)
            ";
            insert.Parameters.AddWithValue("@rowId", row.RowId);
            insert.Parameters.AddWithValue("@name", name);
            insert.Parameters.AddWithValue("@path", row.Path);
            insert.ExecuteNonQuery();
        }
    }

    public void RebuildSearchIndex()
    {
        using var tx = _connection.BeginTransaction();
        try
        {
            RebuildFts(tx);
            tx.Commit();
        }
        catch
        {
            tx.Rollback();
            throw;
        }
    }

    /// <summary>
    /// Read-only detection for legacy rows that cannot be normalized safely or
    /// that collide after native normalization. No repair is guessed here.
    /// </summary>
    public IReadOnlyList<string> InspectProjectPathAnomalies()
    {
        var anomalies = new List<string>();
        var seen = new Dictionary<string, string>(ProjectPathSemantics.NativeComparer);
        using var command = _connection.CreateCommand();
        command.CommandText = "SELECT id, path FROM projects ORDER BY rowid";
        using var reader = command.ExecuteReader();
        while (reader.Read())
        {
            var id = reader.GetString(0);
            var path = reader.GetString(1);
            if (!ProjectPathSemantics.TryNormalizeNative(path, out var normalized))
            {
                anomalies.Add($"Project '{id}' has an invalid legacy path: '{path}'.");
                continue;
            }
            if (seen.TryGetValue(normalized, out var existingId))
            {
                anomalies.Add(
                    $"Projects '{existingId}' and '{id}' collide after path normalization: '{normalized}'.");
                continue;
            }
            seen[normalized] = id;
        }
        return anomalies;
    }

    public List<Project> ListProjects(string? search = null, string? toolKey = null, int limit = 100)
    {
        if (limit is < 1 or > 10000)
            throw new ArgumentOutOfRangeException(
                nameof(limit), limit, "Project limit must be between 1 and 10000.");
        var projects = new List<Project>();
        using var cmd = _connection.CreateCommand();

        if (!string.IsNullOrEmpty(search))
        {
            var normalizedSearch = ProjectPathSemantics.TryNormalizeNative(search, out var candidate)
                && ProjectPathSemantics.GetDisplayName(candidate) == candidate
                ? candidate
                : null;
            if (normalizedSearch is not null)
            {
                var comparison = OperatingSystem.IsWindows()
                    ? "p.path = @root COLLATE NOCASE"
                    : "p.path = @root";
                cmd.CommandText = $@"
                    SELECT p.id, p.path, p.last_accessed_at, p.first_seen_at, p.git_branch, p.git_remote_url
                    FROM projects p
                    WHERE {comparison}
                    ORDER BY p.last_accessed_at DESC
                    LIMIT @limit
                ";
                cmd.Parameters.AddWithValue("@root", normalizedSearch);
            }
            else
            {
                var ftsQuery = BuildFtsPrefixQuery(search);
                if (ftsQuery.Length == 0)
                    return projects;

                // FTS5 MATCH 左侧必须是 FTS 表名（不能用别名），故用子查询取命中 rowid
                cmd.CommandText = @"
                    SELECT p.id, p.path, p.last_accessed_at, p.first_seen_at, p.git_branch, p.git_remote_url
                    FROM projects p
                    WHERE p.rowid IN (
                        SELECT rowid FROM projects_fts WHERE projects_fts MATCH @search
                    )
                    ORDER BY p.last_accessed_at DESC
                    LIMIT @limit
                ";
                cmd.Parameters.AddWithValue("@search", ftsQuery);
            }
        }
        else if (!string.IsNullOrEmpty(toolKey))
        {
            cmd.CommandText = @"
                SELECT DISTINCT p.id, p.path, p.last_accessed_at, p.first_seen_at, p.git_branch, p.git_remote_url
                FROM projects p
                JOIN tool_usages u ON u.project_id = p.id
                WHERE u.tool_key = @toolKey COLLATE NOCASE
                ORDER BY p.last_accessed_at DESC
                LIMIT @limit
            ";
            cmd.Parameters.AddWithValue("@toolKey", toolKey);
        }
        else
        {
            cmd.CommandText = @"
                SELECT id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url
                FROM projects
                ORDER BY last_accessed_at DESC
                LIMIT @limit
            ";
        }
        cmd.Parameters.AddWithValue("@limit", limit);

        using var reader = cmd.ExecuteReader();
        while (reader.Read())
        {
            projects.Add(new Project
            {
                Id = reader.GetString(0),
                Path = reader.GetString(1),
                LastAccessedAt = ReadUtcDateTime(reader, 2),
                FirstSeenAt = ReadUtcDateTime(reader, 3),
                GitBranch = reader.IsDBNull(4) ? null : reader.GetString(4),
                GitRemoteUrl = reader.IsDBNull(5) ? null : reader.GetString(5)
            });
        }

        // 加载 tool usages
        foreach (var p in projects)
        {
            using var uCmd = _connection.CreateCommand();
            uCmd.CommandText = "SELECT tool_name, tool_key, last_used_at, session_count, last_session_id FROM tool_usages WHERE project_id = @pid";
            uCmd.Parameters.AddWithValue("@pid", p.Id);
            using var uReader = uCmd.ExecuteReader();
            while (uReader.Read())
            {
                p.ToolUsages.Add(new ToolUsage
                {
                    ToolName = uReader.GetString(0),
                    ToolKey = uReader.GetString(1),
                    LastUsedAt = ReadUtcDateTime(uReader, 2),
                    SessionCount = uReader.GetInt32(3),
                    LastSessionId = uReader.IsDBNull(4) ? null : uReader.GetString(4)
                });
            }
        }

        return projects;
    }

    public Project? GetProjectByPath(string path)
    {
        if (string.IsNullOrWhiteSpace(path))
            return null;

        if (!ProjectPathSemantics.TryNormalizeNative(path, out var normalizedPath))
            return null;

        using var cmd = _connection.CreateCommand();
        cmd.CommandText = OperatingSystem.IsWindows()
            ? """
                SELECT id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url
                FROM projects
                WHERE path = @path COLLATE NOCASE
                LIMIT 1
                """
            : """
                SELECT id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url
                FROM projects
                WHERE path = @path
                LIMIT 1
                """;
        cmd.Parameters.AddWithValue("@path", normalizedPath);

        using var reader = cmd.ExecuteReader();
        if (!reader.Read())
            return null;

        var project = new Project
        {
            Id = reader.GetString(0),
            Path = reader.GetString(1),
            LastAccessedAt = ReadUtcDateTime(reader, 2),
            FirstSeenAt = ReadUtcDateTime(reader, 3),
            GitBranch = reader.IsDBNull(4) ? null : reader.GetString(4),
            GitRemoteUrl = reader.IsDBNull(5) ? null : reader.GetString(5)
        };
        reader.Close();

        using var usageCommand = _connection.CreateCommand();
        usageCommand.CommandText = """
            SELECT tool_name, tool_key, last_used_at, session_count, last_session_id
            FROM tool_usages
            WHERE project_id = @projectId
            """;
        usageCommand.Parameters.AddWithValue("@projectId", project.Id);
        using var usageReader = usageCommand.ExecuteReader();
        while (usageReader.Read())
        {
            project.ToolUsages.Add(new ToolUsage
            {
                ToolName = usageReader.GetString(0),
                ToolKey = usageReader.GetString(1),
                LastUsedAt = ReadUtcDateTime(usageReader, 2),
                SessionCount = usageReader.GetInt32(3),
                LastSessionId = usageReader.IsDBNull(4) ? null : usageReader.GetString(4)
            });
        }

        return project;
    }

    private static string BuildFtsPrefixQuery(string search)
    {
        var terms = new List<string>();
        var current = new StringBuilder();
        foreach (var character in search)
        {
            if (char.IsLetterOrDigit(character) || character == '_')
            {
                current.Append(character);
                continue;
            }

            if (current.Length == 0)
                continue;
            terms.Add(current.ToString());
            current.Clear();
        }
        if (current.Length > 0)
            terms.Add(current.ToString());

        return string.Join(" AND ", terms.Select(term => $"\"{term}\"*"));
    }

    public void RecordSession(Session session)
    {
        var normalizedPath = ProjectPathSemantics.NormalizeNative(session.ProjectPath);
        using var cmd = _connection.CreateCommand();
        cmd.CommandText = @"
            INSERT INTO sessions (id, project_path, tool_key, tool_name, started_at, ended_at, session_id_from_tool)
            VALUES (@id, @path, @tkey, @tname, @started, @ended, @tsid)
        ";
        cmd.Parameters.AddWithValue("@id", session.Id);
        cmd.Parameters.AddWithValue("@path", normalizedPath);
        cmd.Parameters.AddWithValue("@tkey", session.ToolKey);
        cmd.Parameters.AddWithValue("@tname", session.ToolName);
        cmd.Parameters.AddWithValue("@started", session.StartedAt.ToString("O"));
        cmd.Parameters.AddWithValue("@ended", session.EndedAt?.ToString("O") ?? (object)DBNull.Value);
        cmd.Parameters.AddWithValue("@tsid", session.SessionIdFromTool ?? (object)DBNull.Value);
        cmd.ExecuteNonQuery();
    }

    public List<Session> GetRecentSessions(int limit = 10)
    {
        if (limit is < 1 or > 1000)
            throw new ArgumentOutOfRangeException(
                nameof(limit), limit, "Session limit must be between 1 and 1000.");
        var sessions = new List<Session>();
        using var cmd = _connection.CreateCommand();
        cmd.CommandText = @"
            SELECT id, project_path, tool_key, tool_name, started_at, ended_at, session_id_from_tool
            FROM sessions
            ORDER BY started_at DESC
            LIMIT @limit
        ";
        cmd.Parameters.AddWithValue("@limit", limit);
        using var reader = cmd.ExecuteReader();
        while (reader.Read())
        {
            sessions.Add(new Session
            {
                Id = reader.GetString(0),
                ProjectPath = reader.GetString(1),
                ToolKey = reader.GetString(2),
                ToolName = reader.GetString(3),
                StartedAt = ReadUtcDateTime(reader, 4),
                EndedAt = reader.IsDBNull(5) ? null : ReadUtcDateTime(reader, 5),
                SessionIdFromTool = reader.IsDBNull(6) ? null : reader.GetString(6)
            });
        }
        return sessions;
    }

    private static DateTime ReadUtcDateTime(SqliteDataReader reader, int ordinal)
    {
        var parsed = DateTime.Parse(
            reader.GetString(ordinal),
            CultureInfo.InvariantCulture,
            DateTimeStyles.RoundtripKind);
        return AsUtc(parsed);
    }

    public void Dispose()
    {
        _connection?.Close();
        _connection?.Dispose();
    }
}
