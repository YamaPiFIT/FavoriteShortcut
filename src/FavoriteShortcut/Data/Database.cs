using System.IO;
using Microsoft.Data.Sqlite;

namespace FavoriteShortcut.Data;

/// <summary>
/// SQLite 接続とスキーマ管理。
///
/// スキーマのバージョンは PRAGMA user_version で管理し、起動時に
/// 足りないマイグレーションだけを順番に適用する（§47 のDB互換性要件）。
/// WAL + synchronous=NORMAL により、強制終了時でもDBが壊れにくい構成にしている。
/// </summary>
public sealed class Database : IDisposable
{
    /// <summary>現在のスキーマバージョン。スキーマを変更したら +1 してマイグレーションを追加する。</summary>
    public const int CurrentSchemaVersion = 1;

    private readonly SqliteConnection _connection;

    public string FilePath { get; }

    public Database(string filePath)
    {
        FilePath = filePath;
        var dir = Path.GetDirectoryName(filePath);
        if (!string.IsNullOrEmpty(dir)) Directory.CreateDirectory(dir);

        var csb = new SqliteConnectionStringBuilder
        {
            DataSource = filePath,
            Mode = SqliteOpenMode.ReadWriteCreate,
            Cache = SqliteCacheMode.Private,
            ForeignKeys = true,
        };
        _connection = new SqliteConnection(csb.ToString());
        _connection.Open();

        try
        {
            Exec("PRAGMA journal_mode=WAL;");
            Exec("PRAGMA synchronous=NORMAL;");
            Exec("PRAGMA foreign_keys=ON;");
            Exec("PRAGMA busy_timeout=5000;");

            Migrate();
        }
        catch
        {
            // 初期化に失敗した場合、ファイルを掴んだままにしない
            // （呼び出し側はコンストラクタが投げたインスタンスを Dispose できないため）
            _connection.Dispose();
            SqliteConnection.ClearPool(_connection);
            throw;
        }
    }

    public SqliteConnection Connection => _connection;

    public SqliteCommand CreateCommand(string sql)
    {
        var cmd = _connection.CreateCommand();
        cmd.CommandText = sql;
        return cmd;
    }

    public int Exec(string sql)
    {
        using var cmd = CreateCommand(sql);
        return cmd.ExecuteNonQuery();
    }

    public SqliteTransaction BeginTransaction() => _connection.BeginTransaction();

    public int GetSchemaVersion()
    {
        using var cmd = CreateCommand("PRAGMA user_version;");
        return Convert.ToInt32(cmd.ExecuteScalar() ?? 0);
    }

    private void SetSchemaVersion(int version) => Exec($"PRAGMA user_version={version};");

    private void Migrate()
    {
        var version = GetSchemaVersion();

        if (version > CurrentSchemaVersion)
        {
            throw new InvalidOperationException(
                $"データベースのバージョン ({version}) がこのアプリ ({CurrentSchemaVersion}) より新しいため開けません。" +
                "新しいバージョンのアプリをご使用ください。");
        }

        if (version < 1)
        {
            using var tx = BeginTransaction();
            Exec(SchemaV1);
            tx.Commit();
            SetSchemaVersion(1);
            version = 1;
        }

        // 今後のマイグレーションはここに if (version < 2) { ... } の形で追加する。

        if (version != CurrentSchemaVersion)
            SetSchemaVersion(CurrentSchemaVersion);
    }

    private const string SchemaV1 = """
        CREATE TABLE IF NOT EXISTS folders (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            parent_id   TEXT NULL REFERENCES folders(id) ON DELETE CASCADE,
            sort_order  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS ix_folders_parent ON folders(parent_id, sort_order);

        CREATE TABLE IF NOT EXISTS shortcuts (
            id          TEXT PRIMARY KEY,
            folder_id   TEXT NULL REFERENCES folders(id) ON DELETE SET NULL,
            title       TEXT NOT NULL,
            target      TEXT NOT NULL,
            target_type INTEGER NOT NULL DEFAULT 0,
            icon_path   TEXT NULL,
            note        TEXT NULL,
            sort_order  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS ix_shortcuts_folder ON shortcuts(folder_id, sort_order);
        CREATE INDEX IF NOT EXISTS ix_shortcuts_title  ON shortcuts(title);
        CREATE INDEX IF NOT EXISTS ix_shortcuts_target ON shortcuts(target);

        CREATE TABLE IF NOT EXISTS tags (
            id   TEXT PRIMARY KEY,
            name TEXT NOT NULL COLLATE NOCASE UNIQUE
        );
        CREATE INDEX IF NOT EXISTS ix_tags_name ON tags(name COLLATE NOCASE);

        CREATE TABLE IF NOT EXISTS shortcut_tags (
            shortcut_id TEXT NOT NULL REFERENCES shortcuts(id) ON DELETE CASCADE,
            tag_id      TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
            sort_order  INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (shortcut_id, tag_id)
        );
        CREATE INDEX IF NOT EXISTS ix_shortcut_tags_tag ON shortcut_tags(tag_id);

        CREATE TABLE IF NOT EXISTS shortcut_usage (
            shortcut_id  TEXT PRIMARY KEY REFERENCES shortcuts(id) ON DELETE CASCADE,
            last_used_at TEXT NULL,
            usage_count  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS ix_usage_last ON shortcut_usage(last_used_at DESC);

        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NULL
        );
        """;

    /// <summary>WAL を本体に取り込んでからバックアップ/エクスポートするために使う。</summary>
    public void Checkpoint()
    {
        try { Exec("PRAGMA wal_checkpoint(TRUNCATE);"); }
        catch { /* 失敗してもデータは失われない */ }
    }

    public void Dispose()
    {
        Checkpoint();
        _connection.Dispose();
        SqliteConnection.ClearPool(_connection);
    }
}
