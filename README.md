<p align="center"><img src="docs/hero.png" alt="Agent Vocal Studio · Local vocal editing with MCP" width="100%"></p>

# Agent Vocal Studio

在 Windows 上编辑歌声、编排和混音，让 Agent 通过 MCP 操作同一份可编辑工程。

A local Windows editor for vocal sequencing, multitrack arrangement, and mixing. An MCP client can inspect, preview, and edit the same project that is open on the desktop.

[![Windows 下载](https://img.shields.io/badge/Windows-下载-287CEE?style=flat-square)](https://github.com/Misaoys/agent-vocal-studio/releases/latest) [![MCP stdio](https://img.shields.io/badge/MCP-stdio-6C56CF?style=flat-square)](#mcp-接入) [![MIT License](https://img.shields.io/badge/License-MIT-3A936B?style=flat-square)](LICENSE)

## MCP 接入

**复制这一句话给 Agent**

```text
请按 https://github.com/Misaoys/agent-vocal-studio#mcp-接入 下载并解压 Windows 版，把 bin/agent-vocal-studio.exe 的绝对路径注册为名为 agent_vocal_studio 的 stdio MCP 服务，参数用 --mcp --home <解压根目录的绝对路径>，重载客户端后调用 studio_status 验证连接。
```

如果用 Codex，也可以在解压目录里运行下面的命令。注册后重新打开 Codex。桌面和 MCP 要使用同一个工作区，才能读取桌面选区并控制当前工程的试听。

```powershell
$studioRoot = (Get-Location).Path
$studioExe = Join-Path $studioRoot "bin\agent-vocal-studio.exe"
codex mcp add agent_vocal_studio -- $studioExe --mcp --home $studioRoot
codex mcp get agent_vocal_studio
```

<details>
<summary>其他 stdio MCP 客户端的配置样例</summary>

将以下配置中的两处路径替换为本机的实际解压路径。客户端载入配置后，可以用 `studio_status` 检查服务。

```json
{
  "mcpServers": {
    "agent_vocal_studio": {
      "command": "C:/Apps/Agent Vocal Studio/bin/agent-vocal-studio.exe",
      "args": ["--mcp", "--home", "C:/Apps/Agent Vocal Studio"]
    }
  }
}
```

</details>

## 安装与启动

1. 从 [GitHub Releases](https://github.com/Misaoys/agent-vocal-studio/releases/latest) 下载 `Agent-Vocal-Studio-0.98.0-Windows-x64.zip`，完整解压到可写目录。
2. 双击 `启动歌声编辑器.vbs`，或打开 `bin/agent-vocal-desktop.exe`。界面依赖 Windows 的 [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/#download-section)。
3. 需要采样乐器时，在解压目录运行 `安装乐器音源.ps1`。需要合成歌声时，运行 `安装本地歌声引擎.ps1` 配置 VOICEVOX，或按 [OpenUtau 适配器指南](adapters/openutau/README.md) 配置日语 Classic 声库。音源分别适用各自的条款。

## 编辑器

<p><img src="docs/studio-mixer.png" alt="Agent Vocal Studio 的多轨编排与混音器，显示歌声、钢琴和伴奏轨" width="100%"></p>

| 工作区 | 支持的操作 |
| --- | --- |
| **歌声钢琴卷帘** | 编辑音符、逐音歌词、音高曲线、颤音和跨音延续 |
| **多轨编排** | 编排歌声、复音乐器和音频片段，编辑速度、拍号及段落 |
| **试听与混音** | A/B 试听、音量自动化、轨道效果、编组和共享返回 |
| **交换与交付** | AVPROJ、AVPACK、MIDI、PPSF 曲谱导入预览、WAV 混音和对齐分轨 |

Agent 可以先读取工程版本和选区，再用 `agent_focus` 标出操作目标，预览改动后按版本提交。桌面上可以看到 Agent 正在处理的音轨和范围。 [查看 Agent 选区截图](docs/studio-agent.png)

## 从源码构建

使用 Windows x64、Rust MSVC 工具链及 Visual Studio C++ Build Tools（含 Windows SDK）。

```powershell
git clone https://github.com/Misaoys/agent-vocal-studio.git
cd agent-vocal-studio
powershell -ExecutionPolicy Bypass -File .\build.ps1
cargo test --locked
```

构建脚本把桌面程序和 MCP 服务放入 `bin/`。

## 文档与许可

- [使用手册](docs/使用手册.md) · [工程架构](docs/工程架构设计.md) · [歌声制作与返修](docs/流行歌制作与返修.md)
- 项目源码使用 [MIT 许可](LICENSE)。依赖许可见 [第三方声明](THIRD_PARTY_NOTICES.json)，内置字体使用各自的 [OFL 许可](web/fonts/)。
