using Spectre.Console;
using Spectre.Console.Cli;
using SessionAtlas.Core.Indexer;
using SessionAtlas.Core.Scanner;
using SessionAtlas.Core.Store;
using System.ComponentModel;

namespace SessionAtlas.CLI.Commands;

[Description("扫描所有已知 AI CLI 数据源，更新项目索引")]
public class ScanCommand : AsyncCommand<ScanCommand.Settings>
{
    public class Settings : CommandSettings
    {
        [CommandOption("--tool")]
        [Description("仅扫描指定工具，如 claude, codex, kimi")]
        public string? ToolFilter { get; set; }
    }

    public override async Task<int> ExecuteAsync(CommandContext context, Settings settings)
    {
        var registry = new ScannerRegistry();
        var selected = registry.All
            .Where(scanner =>
                settings.ToolFilter == null ||
                scanner.ToolKey.Equals(settings.ToolFilter, StringComparison.OrdinalIgnoreCase))
            .ToList();

        if (selected.Count == 0)
        {
            if (settings.ToolFilter != null)
            {
                AnsiConsole.MarkupLine(
                    $"[red]未检测到可扫描的工具：{Markup.Escape(settings.ToolFilter)}。[/]");
            }
            else
            {
                AnsiConsole.MarkupLine("[red]未配置任何可扫描的数据源。[/]");
            }
            return 1;
        }

        AnsiConsole.MarkupLine($"[green]将扫描 {selected.Count} 个工具...[/]\n");

        var scanResults = new List<(IProjectScanner Scanner, List<ScannedProject> Results)>();
        var diagnostics = registry.Diagnostics.ToList();
        var skipped = new List<(IProjectScanner Scanner, ScanStatus Status)>();

        await AnsiConsole.Progress()
            .Columns(new ProgressColumn[]
            {
                new TaskDescriptionColumn(),
                new ProgressBarColumn(),
                new PercentageColumn(),
                new RemainingTimeColumn(),
                new SpinnerColumn()
            })
            .StartAsync(async ctx =>
            {
                foreach (var scanner in selected)
                {
                    var task = ctx.AddTask($"[cyan]{Markup.Escape(scanner.ToolName)}[/]", maxValue: 1);
                    try
                    {
                        var outcome = await Task.Run(() => scanner.Scan());
                        diagnostics.AddRange(outcome.Diagnostics);
                        if (outcome.IsSuccessful)
                        {
                            scanResults.Add((scanner, outcome.Projects.ToList()));
                        }
                        else
                        {
                            skipped.Add((scanner, outcome.Status));
                        }
                    }
                    catch (Exception)
                    {
                        diagnostics.Add(new ScanDiagnostic(
                            scanner.ToolKey,
                            ScanDiagnosticSeverity.Error,
                            "unexpected_scanner_failure",
                            "The scanner stopped unexpectedly; its previous index is preserved."));
                        skipped.Add((scanner, ScanStatus.Failed));
                    }
                    finally
                    {
                        task.Value = 1;
                        task.StopTask();
                    }
                }
            });

        foreach (var diagnostic in diagnostics)
        {
            var color = diagnostic.Severity switch
            {
                ScanDiagnosticSeverity.Error => "red",
                ScanDiagnosticSeverity.Warning => "yellow",
                _ => "dim"
            };
            AnsiConsole.MarkupLine(
                $"[{color}]{Markup.Escape(diagnostic.ToolKey)} · " +
                $"{Markup.Escape(diagnostic.Code)}:[/] " +
                Markup.Escape(diagnostic.Message));
        }

        if (scanResults.Count == 0)
        {
            AnsiConsole.MarkupLine("[red]没有工具产生可信快照，索引未发生变化。[/]");
            return 1;
        }

        var totalRaw = scanResults.Sum(r => r.Results.Count);
        AnsiConsole.MarkupLine($"\n[dim]原始扫描结果: {totalRaw} 条[/]");

        var indexer = new ProjectIndexer();
        var projects = indexer.BuildIndex(scanResults);

        AnsiConsole.MarkupLine($"[green]去重合并后: {projects.Count} 个项目[/]");

        using var store = new SqliteStore();
        store.ReplaceToolSnapshots(
            projects,
            scanResults
                .Select(result => result.Scanner.ToolKey)
                .Distinct(StringComparer.OrdinalIgnoreCase)
                .ToArray());

        AnsiConsole.MarkupLine("[green]索引已原子更新到本地数据库。[/]");
        if (skipped.Count > 0)
            AnsiConsole.MarkupLine($"[yellow]{skipped.Count} 个工具保留了上一份索引。[/]");
        return 0;
    }
}
