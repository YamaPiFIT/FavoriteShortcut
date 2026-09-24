using System.IO;
using System.Text.RegularExpressions;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

/// <summary>
/// URL / パス文字列の種類判定と整形（§9）。
///
/// 判定はあくまで「登録時点での推測」であり、対象が存在しない場合でも
/// 登録自体は許可する（§33：DB上の情報を勝手に消さない）。
/// </summary>
public static class TargetResolver
{
    private static readonly Regex SchemeRegex =
        new(@"^(?<scheme>[a-zA-Z][a-zA-Z0-9+.\-]*):", RegexOptions.Compiled);

    private static readonly Regex DriveRegex =
        new(@"^[a-zA-Z]:[\\/]", RegexOptions.Compiled);

    /// <summary>ホスト名らしい文字列（www.example.com / example.co.jp など）。</summary>
    private static readonly Regex BareHostRegex =
        new(@"^[a-zA-Z0-9\-]+(\.[a-zA-Z0-9\-]+)+(/.*)?$", RegexOptions.Compiled);

    private static readonly HashSet<string> ExecutableExtensions = new(StringComparer.OrdinalIgnoreCase)
    {
        ".exe", ".com", ".bat", ".cmd", ".msi", ".lnk", ".ps1", ".vbs",
    };

    /// <summary>入力欄の文字列から対象種別を推測する。</summary>
    public static TargetType Detect(string? target)
    {
        var value = (target ?? string.Empty).Trim();
        if (value.Length == 0) return TargetType.Unknown;

        // UNC パス
        if (value.StartsWith(@"\\", StringComparison.Ordinal))
            return LooksLikeFile(value) ? FileOrApp(value) : TargetType.Folder;

        // ドライブレターつきのローカルパス
        if (DriveRegex.IsMatch(value))
        {
            if (Directory.Exists(value)) return TargetType.Folder;
            if (File.Exists(value)) return FileOrApp(value);
            return LooksLikeFile(value) ? FileOrApp(value) : TargetType.Folder;
        }

        var scheme = SchemeRegex.Match(value);
        if (scheme.Success)
        {
            var name = scheme.Groups["scheme"].Value.ToLowerInvariant();
            return name switch
            {
                "http" or "https" or "ftp" or "ftps" => TargetType.Web,
                "file" => TargetType.File,
                _ => TargetType.Web, // mailto:, ms-settings:, obsidian: などは既定ハンドラに任せる
            };
        }

        // 環境変数を含むパス（%USERPROFILE%\Documents など）
        if (value.Contains('%'))
        {
            var expanded = Environment.ExpandEnvironmentVariables(value);
            if (Directory.Exists(expanded)) return TargetType.Folder;
            if (File.Exists(expanded)) return FileOrApp(expanded);
        }

        if (BareHostRegex.IsMatch(value)) return TargetType.Web;

        return TargetType.Unknown;
    }

    private static TargetType FileOrApp(string path) =>
        ExecutableExtensions.Contains(Path.GetExtension(path)) ? TargetType.Application : TargetType.File;

    private static bool LooksLikeFile(string value)
    {
        var ext = Path.GetExtension(value);
        return ext.Length is > 1 and <= 8 && !ext.Contains(' ');
    }

    /// <summary>
    /// 保存前の整形。"example.com" のようにスキームが無い Web アドレスには https:// を補う。
    /// パスはそのまま保持する（大文字小文字や末尾の \ をユーザーの入力どおりに残す）。
    /// </summary>
    public static string Normalize(string? target, TargetType type)
    {
        var value = (target ?? string.Empty).Trim();
        if (value.Length == 0) return value;

        // 貼り付け時に前後へ付きがちな引用符を落とす
        if (value.Length >= 2 && value[0] == '"' && value[^1] == '"')
            value = value[1..^1].Trim();

        if (type == TargetType.Web && !SchemeRegex.IsMatch(value))
            value = "https://" + value;

        return value;
    }

    /// <summary>対象から既定のタイトル候補を作る（登録ダイアログの補完用）。</summary>
    public static string SuggestTitle(string? target, TargetType type)
    {
        var value = (target ?? string.Empty).Trim();
        if (value.Length == 0) return string.Empty;

        switch (type)
        {
            case TargetType.Web:
                if (Uri.TryCreate(value, UriKind.Absolute, out var uri))
                {
                    var host = uri.Host;
                    if (host.StartsWith("www.", StringComparison.OrdinalIgnoreCase)) host = host[4..];
                    return host.Length > 0 ? host : value;
                }
                return value;

            case TargetType.Folder:
                return Path.GetFileName(value.TrimEnd('\\', '/')) is { Length: > 0 } name ? name : value;

            case TargetType.File:
            case TargetType.Application:
                return Path.GetFileNameWithoutExtension(value) is { Length: > 0 } file ? file : value;

            default:
                return value;
        }
    }

    /// <summary>Web URL を Uri として解釈する。スキームが無い場合だけ https:// を補う。</summary>
    public static Uri? TryGetUri(string? target)
    {
        var value = (target ?? string.Empty).Trim();
        if (value.Length == 0) return null;
        if (!SchemeRegex.IsMatch(value)) value = "https://" + value;

        return Uri.TryCreate(value, UriKind.Absolute, out var uri) && uri.Host.Length > 0 ? uri : null;
    }

    /// <summary>Web URL からホスト名を取り出す。</summary>
    public static string? TryGetHost(string? target) => TryGetUri(target)?.Host;

    /// <summary>
    /// スキーム + ホスト + ポートを取り出す（favicon のキャッシュキー用）。
    ///
    /// ホスト名だけをキーにすると、http と https、あるいはポート違いの別サイトが
    /// 同じアイコンを共有してしまうため、オリジン単位で扱う。
    /// </summary>
    public static string? TryGetOrigin(string? target)
    {
        var uri = TryGetUri(target);
        if (uri is null) return null;

        return uri.IsDefaultPort
            ? $"{uri.Scheme}://{uri.Host}"
            : $"{uri.Scheme}://{uri.Host}:{uri.Port}";
    }

    /// <summary>
    /// 社内・ローカルネットワーク上のホストかどうか。
    ///
    /// 外部のアイコン取得サービスへ問い合わせてよいかの判定に使う。
    /// 社内ホスト名を外部へ送ると命名規則や内部構成が漏れるため、
    /// ここで true になるものは外部サービスの対象から除外する。
    /// </summary>
    public static bool IsPrivateOrIntranetHost(string? host)
    {
        if (string.IsNullOrWhiteSpace(host)) return true;

        host = host.Trim().Trim('[', ']');

        if (System.Net.IPAddress.TryParse(host, out var address))
            return IsPrivateAddress(address);

        // ドットを含まない単一ラベル名（http://portal/ など）は社内ホストとみなす
        if (!host.Contains('.')) return true;

        foreach (var suffix in PrivateSuffixes)
            if (host.EndsWith(suffix, StringComparison.OrdinalIgnoreCase)) return true;

        return false;
    }

    private static readonly string[] PrivateSuffixes =
    {
        ".local", ".localhost", ".localdomain", ".internal", ".intranet",
        ".corp", ".lan", ".home", ".home.arpa", ".private",
        ".test", ".example", ".invalid",
    };

    private static bool IsPrivateAddress(System.Net.IPAddress address)
    {
        if (System.Net.IPAddress.IsLoopback(address)) return true;

        if (address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetwork)
        {
            var b = address.GetAddressBytes();
            return b[0] switch
            {
                10 => true,                                  // 10.0.0.0/8
                127 => true,                                 // 127.0.0.0/8
                169 when b[1] == 254 => true,                // 169.254.0.0/16 リンクローカル
                172 when b[1] >= 16 && b[1] <= 31 => true,   // 172.16.0.0/12
                192 when b[1] == 168 => true,                // 192.168.0.0/16
                _ => false,
            };
        }

        if (address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetworkV6)
        {
            if (address.IsIPv6LinkLocal || address.IsIPv6SiteLocal) return true;
            // fc00::/7 ユニークローカルアドレス
            return (address.GetAddressBytes()[0] & 0xFE) == 0xFC;
        }

        return false;
    }

    /// <summary>環境変数を展開した実際のパス（Web の場合はそのまま）。</summary>
    public static string Expand(string target) =>
        target.Contains('%') ? Environment.ExpandEnvironmentVariables(target) : target;

    /// <summary>対象が実在するか。Web は常に true（オフラインでも登録は有効なため）。</summary>
    public static bool Exists(string target, TargetType type)
    {
        if (type is TargetType.Web or TargetType.Unknown) return true;
        var path = Expand(target);
        return type == TargetType.Folder ? Directory.Exists(path) : File.Exists(path);
    }
}
