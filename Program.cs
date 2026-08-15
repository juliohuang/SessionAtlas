using Spectre.Console.Cli;
using SessionAtlas.CLI.Commands;

namespace SessionAtlas;

class Program
{
    static int Main(string[] args)
    {
        var app = new CommandApp();

        app.Configure(config =>
        {
            config.SetApplicationName("sessionatlas");
            config.SetApplicationVersion("0.1.0");

            config.AddCommand<ScanCommand>("scan")
                .WithDescription("扫描所有已安装的 AI CLI 工具，更新项目索引")
                .WithExample(new[] { "scan", "--tool", "claude" });

            config.AddCommand<ListCommand>("list")
                .WithDescription("列出已索引的所有项目")
                .WithExample(new[] { "list", "-t", "claude", "--interactive" });

            config.AddCommand<SearchCommand>("search")
                .WithDescription("模糊搜索项目")
                .WithExample(new[] { "search", "api" });

            config.AddCommand<OpenCommand>("open")
                .WithDescription("打开项目并启动指定 AI CLI 工具")
                .WithExample(new[] { "open", "--recent" })
                .WithExample(new[] { "open", "~/work/my-api", "-t", "claude" });

            config.AddCommand<RecentCommand>("recent")
                .WithDescription("查看最近会话记录")
                .WithExample(new[] { "recent", "--open" });

            config.AddCommand<ConfigCommand>("config")
                .WithDescription("管理配置和自定义工具")
                .WithExample(new[] { "config", "show" });
        });

        // 无参数时默认行为
        if (args.Length == 0)
        {
            args = new[] { "list", "--interactive" };
        }

        return app.Run(args);
    }
}
