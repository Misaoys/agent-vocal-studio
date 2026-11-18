# 本地 OpenUtau 歌声适配器

Studio 通过独立进程调用 OpenUtau 的 WORLDLINE-R。Rust 负责曲谱、速度、调声、混音、试听、进度和撤销。歌声轨可以分别选本机的日语 Classic 声库或 VOICEVOX 歌手。

## 安装

在 Studio 目录使用 Windows x64 和 .NET 10 SDK 构建。适配器固定使用 OpenUtau 0.1.576 的提交 `bc194b6fe4897427a73066390b286e11205fd10f`。 [OpenUtau 源码](https://github.com/openutau/OpenUtau/tree/bc194b6fe4897427a73066390b286e11205fd10f) · [.NET 下载](https://dotnet.microsoft.com/en-us/download/dotnet/10.0)

```powershell
git clone --depth 1 --branch 0.1.576 https://github.com/openutau/OpenUtau.git runtime/development/openutau/official-0.1.576
$source = (Resolve-Path runtime/development/openutau/official-0.1.576).Path
dotnet publish adapters/openutau/Studio.OpenUtau.csproj -c Release -r win-x64 --self-contained true --output runtime/openutau "-p:OpenUtauSource=$source"
```

将已解包的声库放入 `runtime/openutau/Singers/<声库文件夹>`。文件夹须包含 `character.txt`。用 `runtime/openutau/Studio.OpenUtau.exe catalog` 检查识别出的 ID。然后创建 UTF-8 的 `runtime/openutau/voice.json`。下面以本机测试用的重音 Teto 日语单独音为例。 [官方声库](https://kasaneteto.jp/utau/) · [声库使用条款](https://kasaneteto.jp/guideline/voice.html)

```json
{
  "singer": "重音テト音声ライブラリー",
  "name": "重音テト",
  "phonemizer": "direct",
  "terms_url": "https://kasaneteto.jp/guideline/voice.html"
}
```

`direct` 使用音符上的假名标签。`japanese_presamp` 请求 OpenUtau 的日语 Presamp 分音。其他声库配置保存在 `runtime/openutau/voices/<id>.json`，该文件名是工程引用的配置 ID，主配置的 ID 是 `default`。使用 `direct` 时可指定 `lyric_suffix`，例如重音 Teto 气声音源的 `囁`。后缀会用于合成时匹配标签，歌词仍按原假名显示。发布包包含适配器源码，本机声库按各自许可单独安装。 [重音 Teto 气声音源](https://kasaneteto.jp/utau/)

## 使用

在歌声轨的「歌手」里选择声库，用「工程音源」试听各轨自己的歌手。MCP 用 `singer_list(engine:"openutau")` 查询配置 ID，在 `add_track` 或 `update_track` 写入 `openutau_voice`，传 `null` 时该轨使用 `singer_id`。用 `engine:"project"` 渲染。仍可用 `engine:"openutau"` 统一试听本机主配置。混音回执记录各轨实际使用的引擎、歌手和声库配置，工程包保留配置 ID。换电脑时需配置相同 ID 的声库。

Studio 使用拍与速度表计算实际时间，曲线与颤音采样为 WORLDLINE-R 音高控制，跨音延音作为同一个字演唱。

此引擎使用声库自带的咬字时间。遇到已经设置的 VOICEVOX 子音、元音偏移或绝对音高修正时，渲染会指出具体音符。开放的引擎与声库具有各自的使用条款，当前测试音源指定为重音 Teto。重音 Teto 音源的商用授权范围请查阅官方条款。 [声库使用条款](https://kasaneteto.jp/guideline/voice.html)
