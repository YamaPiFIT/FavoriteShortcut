using System.Diagnostics;
using System.IO;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

public readonly record struct LaunchResult(bool Success, string? ErrorTitle, string? ErrorDetail)
{
    public static LaunchResult Ok() => new(true, null, null);
    public static LaunchResult Fail(string title, string detail) => new(false, title, detail);
}

/// <summary>
/// ショートカットの起動（§10）。
/// Web は既定ブラウザ、フォルダはエクスプローラー、ファイルは既定のアプリで開く。
/// 対象が見つからない場合もDB上のデータには一切手を触れない（§33）。
///
/// 開く処理はすべて、画面とは別の専用スレッド（STA）で行う。
/// Windows は開く処理（ShellExecute）の中で、セキュリティの確認（サイトのゾーン判定、
/// 署名の失効確認、共有フォルダへの問い合わせなど）を行うため、ネットワークの状態が悪いと
/// 数秒〜数十秒待たされることがある。画面のスレッドで行うと、その間アプリが止まり
/// ランチャーのホットキーにも反応しなくなる。確認そのものは省かない（安全性はそのまま）。
/// </summary>
public static class LaunchService
{
    /// <summary>ショートカットを裏のスレッドで開く。UI スレッドから呼び、結果は await で受け取る。</summary>
    public static Task<LaunchResult> LaunchAsync(ShortcutItem item)
    {
        // 項目は画面のスレッドのものなので、必要な値だけを先に取り出しておく
        var target = item.Target;
        var type = item.TargetType;
        return RunOnStaThread(() => Launch(target, type));
    }

    /// <summary>エクスプローラーで対象の場所を、裏のスレッドで開く（ファイルなら選択状態にする）。</summary>
    public static Task<LaunchResult> RevealInExplorerAsync(ShortcutItem item)
    {
        var target = item.Target;
        var type = item.TargetType;
        return RunOnStaThread(() => RevealInExplorer(target, type));
    }

    /// <summary>フォルダやファイルを、裏のスレッドで開く（失敗してもログに残すだけ）。</summary>
    public static void OpenPath(string path)
    {
        _ = RunOnStaThread(() =>
        {
            try
            {
                StartShell(path);
            }
            catch (Exception ex)
            {
                AppLog.Warn($"パスを開けませんでした: {path}", ex);
            }
            return true;
        });
    }

    private static LaunchResult Launch(string? targetText, TargetType itemType)
    {
        var target = (targetText ?? string.Empty).Trim();
        if (target.Length == 0)
            return LaunchResult.Fail(Loc.T("Str.Launch.CannotOpen"), Loc.T("Str.Launch.NoTarget"));

        var type = itemType == TargetType.Unknown ? TargetResolver.Detect(target) : itemType;
        var expanded = TargetResolver.Expand(target);

        try
        {
            switch (type)
            {
                case TargetType.Folder:
                    if (!Directory.Exists(expanded))
                        return LaunchResult.Fail(Loc.T("Str.Launch.FolderNotFound"), expanded);
                    StartShell(expanded);
                    return LaunchResult.Ok();

                case TargetType.File:
                case TargetType.Application:
                    if (!File.Exists(expanded))
                        return LaunchResult.Fail(Loc.T("Str.Launch.FileNotFound"), expanded);
                    StartShell(expanded, workingDirectory: Path.GetDirectoryName(expanded));
                    return LaunchResult.Ok();

                case TargetType.Web:
                    StartShell(TargetResolver.Normalize(expanded, TargetType.Web));
                    return LaunchResult.Ok();

                default:
                    // 種別が判定できないものは、実体があればパスとして、なければそのままシェルに渡す
                    if (Directory.Exists(expanded) || File.Exists(expanded))
                    {
                        StartShell(expanded);
                        return LaunchResult.Ok();
                    }
                    StartShell(expanded);
                    return LaunchResult.Ok();
            }
        }
        catch (Exception ex)
        {
            AppLog.Warn($"起動に失敗: {target}", ex);
            return LaunchResult.Fail(Loc.T("Str.Launch.CannotOpen"), $"{expanded}\n\n{ex.Message}");
        }
    }

    private static LaunchResult RevealInExplorer(string? targetText, TargetType itemType)
    {
        var type = itemType == TargetType.Unknown ? TargetResolver.Detect(targetText) : itemType;
        if (type == TargetType.Web)
            return LaunchResult.Fail(Loc.T("Str.Launch.NotSupported"), Loc.T("Str.Launch.WebHasNoFolder"));

        var expanded = TargetResolver.Expand(targetText ?? string.Empty);

        try
        {
            if (Directory.Exists(expanded))
            {
                Process.Start(new ProcessStartInfo("explorer.exe", $"\"{expanded}\"") { UseShellExecute = true });
                return LaunchResult.Ok();
            }

            if (File.Exists(expanded))
            {
                Process.Start(new ProcessStartInfo("explorer.exe", $"/select,\"{expanded}\"") { UseShellExecute = true });
                return LaunchResult.Ok();
            }

            // 実体が無くても、親フォルダが残っていればそこを開く
            var parent = Path.GetDirectoryName(expanded);
            if (!string.IsNullOrEmpty(parent) && Directory.Exists(parent))
            {
                Process.Start(new ProcessStartInfo("explorer.exe", $"\"{parent}\"") { UseShellExecute = true });
                return LaunchResult.Ok();
            }

            return LaunchResult.Fail(Loc.T("Str.Launch.LocationNotFound"), expanded);
        }
        catch (Exception ex)
        {
            AppLog.Warn($"エクスプローラー表示に失敗: {expanded}", ex);
            return LaunchResult.Fail(Loc.T("Str.Launch.CannotOpen"), $"{expanded}\n\n{ex.Message}");
        }
    }

    private static void StartShell(string target, string? workingDirectory = null)
    {
        var psi = new ProcessStartInfo(target) { UseShellExecute = true };
        if (!string.IsNullOrEmpty(workingDirectory) && Directory.Exists(workingDirectory))
            psi.WorkingDirectory = workingDirectory;
        Process.Start(psi);
    }

    /// <summary>
    /// 処理を専用のスレッド（STA）で行う。ShellExecute は COM を使うので STA で呼ぶ。
    /// 1 件ごとにスレッドを分けるので、応答の遅い共有フォルダを開いている間も、次に開くものは待たされない。
    /// </summary>
    internal static Task<T> RunOnStaThread<T>(Func<T> work)
    {
        var done = new TaskCompletionSource<T>(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread = new Thread(() =>
        {
            try
            {
                done.TrySetResult(work());
            }
            catch (Exception ex)
            {
                done.TrySetException(ex);
            }
        })
        {
            IsBackground = true,
            Name = "LaunchService",
        };
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        return done.Task;
    }
}
