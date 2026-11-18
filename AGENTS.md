# Agent Vocal Studio 工作规则

本目录是独立的 Rust 音乐编辑器。当前用户要求优先使用它的结构化 MCP/CLI；旧目录的 REAPER/Piapro 默认实现不覆盖本项目的软件架构与歌声引擎选择。原工程及软件安装保持独立。

- 核心逻辑使用 Rust；桌面窗口内用 WebView2 绘制 HTML/CSS/Canvas。启动入口为 Windows EXE。Agent 使用 MCP/CLI。不得以鼠标盲点代替已存在的工程接口。
- Agent 修改绑定工程 ID、版本与稳定音符 ID。指定范围优先使用 selection_resolve/selection_edit，读取人框选范围用 from_ui。
- 编辑前用 agent_focus 解释具体目标。专属光标是界面绘制，不控制操作系统鼠标；人输入时不抢焦点。
- 将 agent_focus 回执中的 intent_id 与同一工程版本传给 project_edit、selection_edit 或 render_start。一个选区的预览与提交可以使用同一 ID。提交后如需继续处理新版本，重新读取并说明下一项任务。
- 新歌先读 music_knowledge 和 instrument_catalog，保存具体 composition_design，再先制作可听的核心乐句和简伴奏。不能用随机音符或套用四句固定模板冒充成熟作品。
- 技术测试必须明确标为 technical_test，限短片段；不能当作交付歌曲。用户认可的旋律或文本应保留到其明确要求修改。
- 风格按律动、句法、和声、配器、演唱与空间理解；参照写具体职责，不把风格简化为 BPM。内置知识不等于模型训练或覆盖全部风格。
- 报告只陈述实际执行和测量结果。composition_review 是复核提示，不是好听评分。没有真实听审时不得声称已听过或用户满意。
- 改重复钩子前选定参照的首尾音符，通过 motif_returns 对照当前允许范围的回归。把保留的开头与只改这一次的词或句尾落实到具体音符。
- 比较同一版混音中两个乐句时，用 range_audition 绑定现有 job_id 和两个拍区间，再用带票据的 desktop_transport audition_side 播放及切换。听完释放相同的 audition_id。
- 比较两个已完成混音时，给 range_audition 传 renders 和两边的拍区间。至少一边须对应桌面打开的工程版本。
- 乐器轨使用实际音色目录，允许复音；逐音歌词、音高曲线和颤音属于歌声。踏板、表情等使用支持的 MIDI CC；不静默忽略不支持参数。
- 渲染保存 job_id 并查询阶段。超时先 render_jobs 查回执，不盲目重提。取消请求不等于停止；等待 cancelled，绝不终止共用歌声引擎。
- 原生工程/工程包保留可编辑曲谱和素材。导入新建副本，保存前核对路径；冲突后读回，不盲目重放写命令。
- UI 偏 Apple：平面分区、SVG 图标和短文字，少副标题，不做卡片套卡片。动效集中配置，拖拽和播放不增加时钟延迟，支持减少动态效果。
- 完整 DAW 的设计项和已实现项要分开写；实时音频、录音与插件宿主未经专项验证不得称为可用。
- 保护所有工作区之外的用户工程。开发测试使用独立 fixture；禁止全盘清理、杀掉其他音乐软件进程或修改音源授权。

- 速度与拍号先读 timeline_query。原生工程的拍是四分音符拍，变速下不能用起始 BPM 换算秒数。改变速度或拍号时使用带版本的 set_project，读取时间点和实际播放结果。
- 多轨段落操作先读 arrangement_query，把 beat_range 和 track_id 交给 agent_focus 的 arrangement 区域。arrangement_edit 先预览，再用同一请求和 preview_id 提交。注意目标段仍使用自身速度和 MIDI 控制，复制后的演唱与编配应重新试听。
- 编辑段落标记时，先通过 arrangement_query 读取当前段落，再用 project_preview 和带版本的 project_edit 提交 set_project.values.section_markers。每项含 name、start、end，可附 role。列表按起拍排序且互不重叠。空列表清除标记，null 恢复显示作品设计中的段落。标记和作品设计分别保存。
- 伴奏剪辑用 audio_clips 和 audio_clip_edit。源帧为 48 kHz，工程位置为四分音符拍。旧音频的 clip_id 在首次剪辑前是 legacy，之后用回执里的稳定 ID。修改同轨重叠须明确选择，成片导出须核对接缝和电平。
- 桌面范围试听用 desktop_transport。先读 status，确认窗口显示同一工程和版本，用同版本完成的渲染 job_id 和具体拍范围播放。默认复听一次。用户接管、超时或控制凭据失效时先读状态；只在合理的当前操作中申请新凭据。设备回调仅证明播放过程，音乐评价需实际听审。
- 跨轨片段编辑用 audio_clip_group。读取用户片段框选用 audio_clips 的 from_ui。可从用户选区生成预览，提交要带预览返回的明确片段列表。变速导致碰撞时重选目标，避免放宽范围或重放旧版本命令。
- 在同一拍切分多轨片段时，先根据片段的工程拍范围列出游标实际穿过的片段，再用 audio_clip_group 的 split 预览和提交。每个目标都必须跨过切点。淡化区拒绝后，调整切点并重新预览。
- 要先听切点时，用 arrangement_audition 的 clip_group 编辑类型和同一个 preview_id，沿用同一份 clips 与 split 参数。默认试听切点前后，确认后应用已试听的候选工程。
- 混音先读 effect_catalog，再按当前工程版本写指定轨和总线的效果器。参数调整后核对新渲染的峰值、分轨与限制器报告。采样峰值和技术信号测量不替代听审。
- 共用空间时可让指定轨通过 sends 送往同一 aux_buses 返回。比较发送量与返回尾音的听感后再应用；混响、延迟作为共享空间时用全湿设置。删除返回和清除引用放在同一带版本编辑批次。
- 鼓、和声等需要共同处理时，用 mix_groups 与各轨 output_group 分配，保留轨内平衡。组压缩前先核对信号电平和峰值，再比较整组的起音与尾音。删除组时同批将成员重定向到总线或其他组。
- 导出响度与峰值取 render_status 的 loudness。按项目目标核对 integrated_lufs 与 true_peak_dbtp，峰值秒位用于定位复听。用 LUFS 匹配比较音色和动态，用原电平比较音量改动。
- 真峰值限制设 master_limiter.peak_mode 为 true_peak，在当前工程版本导出前后比较衰减和响度。试听使用渲染的 WAV。用最终报告核对所需 dBTP 上限。
- 主唱或底鼓需要局部让位时，可在指定目标的效果链添加 ducker。先用 effect_catalog 确认前置触发口，再按 source_track_id 绑定具体轨道。比较触发前、触发时和释放后的同段音频，并用 ducking_max_reduction_db 核对处理深度。无声触发轨仍按其实际谱面或片段驱动。
- 检查主唱与伴奏的电平冲突时，用 render_balance 读取当前版本已完成的分轨原电平。先定位相对中频接近的拍段，再用同版 A/B 复听咬字、音色和空间。数字用于找位置，写听审结论时说明实际听到的内容。
