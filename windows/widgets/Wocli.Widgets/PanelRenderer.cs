using System.Net;
using Wocli.Widgets.Core;

namespace Wocli.Widgets;

internal static class PanelRenderer
{
    public static string Render(RuntimeSnapshot state)
    {
        static string Encode(string? value) => WebUtility.HtmlEncode(value ?? "");
        var message = state.Enabled
            ? "Windows 决定任务栏公告是否显示及显示时长。此预览不保证常驻或逐句同步。"
            : "系统歌词预览已关闭或尚未启用。";
        var status = state.State switch
        {
            "starting" => "正在启动",
            "disabled" => "已关闭",
            "stopped" => "已停止",
            "offline" or "unavailable" => "暂时无法连接播放器",
            "paused" => "已暂停",
            "playing" => "正在播放",
            "boss" => "已隐藏",
            "switching_source" => "正在切换播放器",
            "waiting" => "等待播放",
            "waiting_for_lyrics" => "等待歌词",
            "requested" => "等待系统显示",
            "shown" => "系统已确认显示",
            _ => "等待更新"
        };
        return $$"""
            <!doctype html>
            <html lang="zh-CN">
            <head>
              <meta charset="utf-8">
              <meta name="viewport" content="width=device-width, initial-scale=1">
              <meta name="color-scheme" content="light dark">
              <meta http-equiv="refresh" content="2;url=/lyrics">
              <title>wocli 歌词</title>
              <style>
                :root { color-scheme: light dark; font-family: system-ui; }
                body { background: Canvas; color: CanvasText; margin: 1rem; overflow-wrap: anywhere; }
                h1 { font-size: 1.2em; }
                blockquote { margin-inline: 0; }
              </style>
            </head>
            <body>
              <h1>wocli 歌词 · 系统组件预览</h1>
              <p>{{Encode(message)}}</p>
              <h2>{{Encode(state.Title)}}</h2>
              <p>{{Encode(state.Artist)}}</p>
              <blockquote>{{Encode(state.Line)}}</blockquote>
              <p>状态：{{Encode(status)}}</p>
              <p>最近系统展示确认：{{Encode(state.LastShownAt?.ToLocalTime().ToString("G") ?? "尚未收到")}}</p>
              <p>本面板仅从本机读取播放状态，不将歌曲信息发送到外网。</p>
            </body>
            </html>
            """;
    }
}
