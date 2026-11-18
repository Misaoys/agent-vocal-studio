use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering::SeqCst},
        mpsc, Arc,
    },
    thread,
};
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::windows::WindowExtWindows,
    window::{Icon, ResizeDirection, WindowBuilder},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE},
};
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};
const ORIGIN: &str = "http://avs.studio/";
enum DesktopEvent {
    Ipc(Value),
    Reply(u64, std::result::Result<Value, String>),
    Files(Vec<PathBuf>),
    AudioUpdate(Value),
    RecordingClosed(u64, Value),
}
struct Instance(HANDLE);
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn source_export(root: &Path, url: &str) -> Result<PathBuf> {
    ensure!(
        url.starts_with("/exports/"),
        "Only completed application exports can be saved"
    );
    let relative = crate::service::unescape(url.trim_start_matches('/'))?;
    let path = Path::new(&relative);
    ensure!(
        path.components()
            .all(|c| matches!(c, std::path::Component::Normal(_))),
        "Invalid export path"
    );
    let source = root.join(path).canonicalize()?;
    ensure!(
        source.is_file() && source.starts_with(root.join("exports").canonicalize()?),
        "Export outside project home"
    );
    Ok(source)
}
fn source_mix(root: &Path, url: &str) -> Result<PathBuf> {
    let path = source_export(root, url)?;
    ensure!(
        path.file_name().and_then(|name| name.to_str()) == Some("试听混音.wav"),
        "请选择已渲染的试听混音"
    );
    Ok(path)
}
fn save_copy(source: &Path, target: &Path, replace: bool) -> Result<()> {
    if target.canonicalize().is_ok_and(|path| path == source) {
        return Ok(());
    }
    let temporary = target.with_file_name(crate::model::uid("avs_export") + ".tmp");
    let result = (|| {
        let mut input = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        crate::model::move_file_windows(&temporary, target, replace)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn file_action(root: &Path, op: &str, args: &Value) -> Result<Value> {
    if op == "file_browse" {
        return crate::file_browser::browse(root, args);
    }
    let source = source_export(root, crate::model::field(args, "url")?)?;
    if op == "export_name" {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .context("导出文件名不可显示")?;
        return Ok(json!({"name":name}));
    }
    let target = crate::file_browser::target(args, &source)?;
    let replace = args["replace"] == true;
    if target.exists() && !replace {
        return Ok(json!({"occupied":true}));
    }
    save_copy(&source, &target, replace)?;
    Ok(json!({"path":crate::file_browser::display(&target)}))
}

fn window_control(
    window: &tao::window::Window,
    args: &Value,
    move_origin: &mut Option<(PhysicalPosition<i32>, f64)>,
) -> Result<Value> {
    match crate::model::field(args, "action")? {
        "state" => (),
        "minimize" => window.set_minimized(true),
        "maximize" => window.set_maximized(!window.is_maximized()),
        "move_start" => {
            *move_origin = if window.is_maximized() {
                None
            } else {
                Some((window.outer_position()?, window.scale_factor()))
            };
        }
        "move_drag" => {
            if let Some((origin, scale)) = move_origin {
                let offset = |field: &str| -> Result<i32> {
                    let delta = args[field]
                        .as_f64()
                        .filter(|n| n.is_finite() && n.abs() <= 30_000.)
                        .context("Invalid drag movement")?;
                    Ok((delta * *scale).round() as i32)
                };
                window.set_outer_position(PhysicalPosition::new(
                    origin.x.saturating_add(offset("dx")?),
                    origin.y.saturating_add(offset("dy")?),
                ));
            }
        }
        "move_end" => *move_origin = None,
        "resize" => {
            let direction = match crate::model::field(args, "edge")? {
                "n" => ResizeDirection::North,
                "s" => ResizeDirection::South,
                "e" => ResizeDirection::East,
                "w" => ResizeDirection::West,
                "ne" => ResizeDirection::NorthEast,
                "nw" => ResizeDirection::NorthWest,
                "se" => ResizeDirection::SouthEast,
                "sw" => ResizeDirection::SouthWest,
                _ => anyhow::bail!("Unknown resize edge"),
            };
            if !window.is_maximized() {
                window.drag_resize_window(direction)?;
            }
        }
        _ => anyhow::bail!("Unknown window control"),
    };
    let position = window.outer_position().ok().map(|point| [point.x, point.y]);
    Ok(
        json!({"maximized":window.is_maximized(),"shadow":window.has_undecorated_shadow(),"position":position,"scale":window.scale_factor()}),
    )
}

fn reply_script(id: u64, result: std::result::Result<Value, String>) -> String {
    let value = match result {
        Ok(data) => json!({"ok":true,"data":data}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    format!(
        "window.__desktopReply?.({id},{});",
        serde_json::to_string(&value).unwrap()
    )
}
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = if let Some(i) = args.iter().position(|a| a == "--home") {
        PathBuf::from(args.get(i + 1).context("--home needs a directory")?)
    } else {
        std::env::current_exe()?
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    };
    let root = root.canonicalize()?;
    let startup_project = args
        .iter()
        .position(|arg| arg == "--project")
        .map(|index| {
            let id = args
                .get(index + 1)
                .context("--project needs a project ID")?;
            crate::model::identifier(id)?;
            Ok::<_, anyhow::Error>(id)
        })
        .transpose()?;
    fs::create_dir_all(root.join("runtime"))?;
    let mutex_name = wide(&format!(
        "Local\\AgentVocalStudio_{:x}",
        Sha256::digest(root.to_string_lossy().to_lowercase().as_bytes())
    ));
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    ensure!(!handle.is_null(), "Cannot create desktop instance handle");
    let existing = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _instance = Instance(handle);
    if existing {
        for _ in 0..20 {
            if let Ok(bytes) = fs::read(root.join("runtime/desktop-session.json")) {
                if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                    let hwnd = v["hwnd"].as_u64().unwrap_or(0) as usize as *mut std::ffi::c_void;
                    if unsafe { IsWindow(hwnd) } != 0 {
                        unsafe {
                            ShowWindow(hwnd, SW_RESTORE);
                            SetForegroundWindow(hwnd);
                        }
                        return Ok(());
                    }
                }
            }
            thread::sleep(std::time::Duration::from_millis(100));
        }
        return Ok(());
    }
    crate::service::prepare_desktop(&root)?;
    let event_loop = EventLoopBuilder::<DesktopEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let verify = args.iter().any(|arg| arg == "--verify-ui");
    let verify_theme = args.iter().any(|arg| arg == "--verify-theme");
    use crate::transport_link::Request;
    let (audio_commands, audio_requests) = mpsc::sync_channel(32);
    let capture_transport = audio_commands.clone();
    let (record_commands, record_requests) = mpsc::sync_channel(16);
    let capture_home = root.clone();
    let capture_proxy = proxy.clone();
    thread::Builder::new()
        .name("audio-capture".into())
        .spawn(move || {
            crate::recording::run(
                capture_home,
                record_requests,
                verify,
                capture_transport,
                |id, closed, result| {
                    let event = if closed {
                        DesktopEvent::RecordingClosed(
                            id,
                            result.expect("capture closed with success"),
                        )
                    } else {
                        DesktopEvent::Reply(id, result.map_err(|error| format!("{error:#}")))
                    };
                    capture_proxy.send_event(event).is_ok()
                },
            );
        })?;
    let audio_link = crate::transport_link::listen(&root, audio_commands.clone())?;
    let audio_proxy = proxy.clone();
    let audio_home = root.clone();
    let audio_epoch = Arc::new(AtomicU64::new(0));
    let worker_epoch = audio_epoch.clone();
    let (monitor_signal, monitor_queue) = mpsc::sync_channel(1);
    let monitor_slot = Arc::new(std::sync::Mutex::new(None::<crate::live_mix::Work>));
    let latest_monitor = monitor_slot.clone();
    let monitor_home = root.clone();
    let monitor_reply = audio_commands.clone();
    thread::Builder::new()
        .name("monitor-prepare".into())
        .spawn(move || {
            while monitor_queue.recv().is_ok() {
                let Some(work) = latest_monitor.lock().unwrap().take() else {
                    continue;
                };
                let control = work.control().clone();
                if control.check().is_err() {
                    continue;
                }
                let result = work.run(&monitor_home);
                if monitor_reply
                    .send(Request::MonitorReady { control, result })
                    .is_err()
                {
                    break;
                }
            }
        })?;
    thread::Builder::new()
        .name("audio-transport".into())
        .spawn(move || {
            let mut transport = crate::transport_link::Controller::new(&audio_home, worker_epoch);
            while let Ok(request) = audio_requests.recv() {
                let resolve = |url: &str| source_mix(&audio_home, url);
                match request {
                    Request::Capture { command, reply } => {
                        let result = transport.capture(&audio_home, command, resolve);
                        if let Ok(status) = &result {
                            let _ =
                                audio_proxy.send_event(DesktopEvent::AudioUpdate(status.clone()));
                        }
                        let _ = reply.send(result);
                    }
                    Request::User(id, args) => {
                        let result = transport
                            .user(&audio_home, &args, resolve, |work| {
                                *monitor_slot.lock().unwrap() = Some(work);
                                match monitor_signal.try_send(()) {
                                    Ok(()) | Err(mpsc::TrySendError::Full(())) => Ok(()),
                                    Err(mpsc::TrySendError::Disconnected(())) => {
                                        monitor_slot.lock().unwrap().take();
                                        anyhow::bail!("监听准备线程已退出")
                                    }
                                }
                            })
                            .map_err(|error| format!("{error:#}"));
                        if audio_proxy
                            .send_event(DesktopEvent::Reply(id, result))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Request::MonitorReady { control, result } => {
                        match transport.monitor_ready(&audio_home, &control, result) {
                            Ok(Some(status)) => {
                                if audio_proxy
                                    .send_event(DesktopEvent::AudioUpdate(status))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Ok(None) => (),
                            Err(error) => eprintln!("Updating the monitor: {error:#}"),
                        }
                    }
                    Request::Agent {
                        command,
                        expires,
                        reply,
                    } => {
                        let publish = !command.read_only();
                        let result = transport.agent(&audio_home, command, expires, resolve);
                        if publish {
                            if let Ok(status) = &result {
                                let _ = audio_proxy
                                    .send_event(DesktopEvent::AudioUpdate(status.clone()));
                            }
                        }
                        let _ = reply.send(result);
                    }
                }
            }
        })?;
    let geometry = fs::read(root.join("runtime/window.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .unwrap_or(json!({}));
    let width = geometry["width"]
        .as_f64()
        .unwrap_or(1440.)
        .clamp(1100., 3840.);
    let height = geometry["height"]
        .as_f64()
        .unwrap_or(940.)
        .clamp(720., 2160.);
    let window = WindowBuilder::new()
        .with_title("Agent Vocal Studio")
        .with_visible(false)
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(width, height))
        .with_min_inner_size(LogicalSize::new(1100., 720.))
        .with_maximized(geometry["maximized"].as_bool().unwrap_or(false))
        .with_window_icon(Some(Icon::from_rgba(
            include_bytes!("../resources/app.rgba").to_vec(),
            64,
            64,
        )?))
        .build(&event_loop)?;
    window.set_undecorated_shadow(true);
    let profile = if verify { "webview-verify" } else { "webview" };
    let mut context = WebContext::new(Some(root.join("runtime").join(profile)));
    let initialization_script = format!(
        "{}\nObject.defineProperty(window,'studioStartupProject',{{value:{}}});",
        include_str!("../web/desktop-bridge.js"),
        json!(startup_project)
    );
    let diagnostic_origin = root.join("verification/desktop_message_origin.txt");
    let diagnostic_page = root.join("verification/desktop_page_load.txt");
    let ipc_proxy = proxy.clone();
    let drop_proxy = proxy.clone();
    let webview=WebViewBuilder::new_with_web_context(&mut context).with_url(ORIGIN).with_browser_accelerator_keys(false).with_default_context_menus(false).with_hotkeys_zoom(false)
        .with_custom_protocol("avs".into(), |_,request| {
            let result=crate::service::editor_asset(request.uri().path(),"");
            let (status,content,bytes)=match result{Ok((bytes,content))=>(200,content,bytes),Err(_)=>(404,"text/plain",b"Not found".to_vec())};
            wry::http::Response::builder().status(status).header("Content-Type",content)
                .header("Content-Security-Policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'none'; object-src 'none'; frame-src 'none'; base-uri 'none'")
                .body(bytes.into()).unwrap()
        }).with_clipboard(true).with_devtools(false).with_autoplay(true)
        .with_initialization_script(&initialization_script)
        .with_on_page_load_handler(move |event,url|{if verify&&matches!(event,wry::PageLoadEvent::Finished){let _=fs::write(&diagnostic_page,url);}})
        .with_ipc_handler(move |request| {
            let origin = request.uri().to_string();
            if verify {
                let _ = fs::write(&diagnostic_origin, &origin);
            }
            if origin.trim_end_matches('/') == ORIGIN.trim_end_matches('/') {
                if let Ok(value) = serde_json::from_str::<Value>(request.body()) {
                    let _ = ipc_proxy.send_event(DesktopEvent::Ipc(value));
                }
            }
        })
        .with_navigation_handler(|url|url==ORIGIN||url==ORIGIN.trim_end_matches('/'))
        .with_new_window_req_handler(|_,_|wry::NewWindowResponse::Deny)
        .with_download_started_handler(|_,_|false)
        .with_drag_drop_handler(move |event|{if let wry::DragDropEvent::Drop{paths,..}=event{let _=drop_proxy.send_event(DesktopEvent::Files(paths));}true})
        .build(&window)?;
    let session = json!({"pid":std::process::id(),"hwnd":window.hwnd() as usize,"version":env!("CARGO_PKG_VERSION"),"mode":"desktop","chrome":"studio","renderer":"WebView2","agent_transport":"native MCP","root":root});
    fs::write(
        root.join("runtime/desktop-session.json"),
        serde_json::to_vec_pretty(&session)?,
    )?;
    window.set_visible(true);
    let mut verified = false;
    let mut move_origin = None;
    event_loop.run(move |event,_,flow|{
        let _keep_alive=(&_instance,&context,&audio_link);*flow=ControlFlow::Wait;
        match event {
            Event::WindowEvent{event:WindowEvent::CloseRequested,..}=>{let _=webview.evaluate_script("if(window.desktopRequestClose){window.desktopRequestClose()}else{window.ipc.postMessage(JSON.stringify({op:'close',id:0}))}");},
            Event::WindowEvent{event:WindowEvent::Resized(_),..}=>{let size=window.inner_size().to_logical::<f64>(window.scale_factor());if size.width>0.&&size.height>0.{let _=fs::write(root.join("runtime/window.json"),json!({"width":size.width,"height":size.height,"maximized":window.is_maximized()}).to_string());}},
            Event::UserEvent(DesktopEvent::Files(paths))=>{let _=webview.evaluate_script(&format!("window.desktopImportPaths?.({});",json!(paths)));},
            Event::UserEvent(DesktopEvent::Reply(id,result))=>{let _=webview.evaluate_script(&reply_script(id,result));},
            Event::UserEvent(DesktopEvent::AudioUpdate(status))=>{let _=webview.evaluate_script(&format!("window.desktopAudioUpdate?.({status});"));},
            Event::UserEvent(DesktopEvent::RecordingClosed(id,status))=>{
                audio_link.unpublish();
                let _=webview.evaluate_script(&reply_script(id,Ok(status)));
                *flow=ControlFlow::Exit;
            },
            Event::UserEvent(DesktopEvent::Ipc(value))=>{
                let id=value["id"].as_u64().unwrap_or(0);let op=value["op"].as_str().unwrap_or("");let arguments=value["args"].clone();
                let immediate:Option<Result<Value>>=match op {
                    "ready"=>{
                        window.set_visible(true);
                        if !verified {
                            verified=true;
                            if verify_theme { let _=webview.evaluate_script(include_str!("../verification/theme_probe.js")); }
                            else if verify { let _=webview.evaluate_script(include_str!("../verification/desktop_probe.js")); }
                        }
                        Some(Ok(json!(true)))
                    },
                    "close"=>match record_commands.try_send(crate::recording::Request::Close(id)){
                        Ok(())=>None,
                        Err(_)=>Some(Err(anyhow::anyhow!("录音控制暂时不可用"))),
                    },
                    "recording"=>match record_commands.try_send(crate::recording::Request::Call(id,arguments)){
                        Ok(())=>None,
                        Err(_)=>Some(Err(anyhow::anyhow!("录音控制暂时不可用"))),
                    },
                    "host_info"=>Some(Ok(session.clone())),
                    "transport"=>{
                        if !matches!(arguments["command"].as_str(), Some("status" | "loop_sync" | "output_devices")) {
                            audio_epoch.fetch_add(1,SeqCst);
                        }
                        match audio_commands.try_send(Request::User(id,arguments)) {
                            Ok(())=>None,
                            Err(_)=>Some(Err(anyhow::anyhow!("音频控制暂时不可用"))),
                        }
                    },
                    "title"=>{if let Some(title)=arguments["title"].as_str(){window.set_title(&format!("{} — Agent Vocal Studio",title.chars().take(200).collect::<String>()));}Some(Ok(json!(true)))},
                    "verification_result" if verify=>Some(fs::write(root.join("verification/desktop_probe_report.json"),serde_json::to_vec_pretty(&arguments).unwrap()).map(|_|json!(true)).map_err(Into::into)),
                    "verification_theme_result" if verify_theme=>Some(fs::write(root.join("verification/theme_report.json"),serde_json::to_vec_pretty(&arguments).unwrap()).map(|_|json!(true)).map_err(Into::into)),
                    "verification_long_mix" if verify=>Some((|| {
                        let receipt:Value=serde_json::from_slice(&fs::read(root.join("runtime/pocket_weather.json"))?)?;
                        Ok(json!({"project_id":receipt["project_id"],"job_id":receipt["final_job"]}))
                    })()),
                    "verification_live_stream" if verify=>Some((|| {
                        let receipt:Value=serde_json::from_slice(&fs::read(root.join("verification/live_stream_report.json"))?)?;
                        Ok(receipt)
                    })()),
                    "verification_audition" if verify=>Some((|| {
                        let (pair,paths)=crate::audition::read(&root,crate::model::field(&arguments,"audition_id")?)?;
                        let a=crate::transport::read_mix(&paths[0])?;
                        let b=crate::transport::read_mix(&paths[1])?;
                        let level=|frames:&[[f32;2]]| {
                            let power=frames.iter().flatten().map(|v|f64::from(*v).powi(2)).sum::<f64>()/(frames.len()*2) as f64;
                            10.*power.log10()
                        };
                        let [fa,fb]=pair.frame_ranges;
                        let distinct=a!=b;
                        let saved_mix_matches=arguments["reference_job_id"].as_str().map(|job| -> Result<bool> {
                            let directory=crate::audition::export_directory(&root,job)?;
                            let name=if let Some(output)=&pair.output{
                                let project=crate::model::Store::new(root.clone())?.read(&pair.project_id)?;
                                ensure!(project.revision==pair.revision,"Comparison project revision changed");
                                let result=crate::audition::completed_result(&root,&project,job)?;
                                crate::audition::output_files(&project,&result,Some(std::slice::from_ref(output)))?
                                    .into_iter().next().context("Missing exported output")?
                            }else{"试听混音.wav".into()};
                            let mut wav=crate::monitor_stream::open_mix(&directory.join(name))?;
                            wav.seek(fa[0] as u32)?;
                            Ok(crate::monitor_stream::read_frames(&mut wav,fa[1]-fa[0])?==a)
                        }).transpose()?;
                        let hash_window=arguments.get("hash_window").map(|arg|
                            serde_json::from_value::<[usize;2]>(arg.clone())).transpose()?;
                        let mut pcm_hashes=Vec::new();
                        if hash_window.is_some() || arguments["hash_audio"]==true {
                            for (samples, [start,end]) in [(&a,fa),(&b,fb)] {
                                let [first,last]=hash_window.unwrap_or([start,end]);
                                ensure!(first>=start&&last>first&&last<=end,"Verification PCM window is outside the pair");
                                let mut hash=Sha256::new();
                                for sample in samples[first-start..last-start].iter().flatten() {
                                    hash.update(sample.to_le_bytes());
                                }
                                pcm_hashes.push(format!("{:x}",hash.finalize()));
                            }
                        }
                        let true_peaks=if arguments["measure_true_peak"]==true{
                            [&a,&b].map(|frames|{
                                let mut detector=crate::loudness::TruePeak::default();
                                for frame in frames { detector.push(*frame); }
                                20.*f64::from(detector.flush()).log10()
                            })
                        }else{[f64::NEG_INFINITY;2]};
                        let sample_peaks=(arguments["measure_sample_peak"]==true).then(||
                            [&a,&b].map(|frames|{
                                let peak=frames.iter().flatten().fold(0_f32,|value,sample|value.max(sample.abs()));
                                (peak>=1e-6).then(||20.*f64::from(peak).log10())
                            }));
                        let levels=if pair.match_method==crate::audition::MatchMethod::Lufs{
                            [
                                crate::loudness::Loudness::scan(&a,||Ok(()))?.integrated(1.),
                                crate::loudness::Loudness::scan(&b,||Ok(()))?.integrated(1.)
                            ]
                        }else{[None;2]};
                        let mut region_peaks=Vec::new();
                        if let Some(regions)=arguments["candidate_regions"].as_array() {
                            let clock=pair.timing[1].clock();
                            for region in regions.iter().take(8) {
                                let frame=|value:&Value| -> Result<usize> {
                                    let beat=value.as_f64().context("Missing verification beat")?;
                                    Ok(((clock.seconds(beat)*48000.).round() as usize).saturating_sub(fb[0]).min(b.len()))
                                };
                                let start=frame(&region[0])?;
                                let end=frame(&region[1])?;
                                region_peaks.push(b.get(start..end).unwrap_or_default().iter().flatten()
                                    .fold(0_f32,|peak,sample|peak.max(sample.abs())));
                            }
                        }
                        Ok(json!({"frames":[a.len(),b.len()],"expected":[fa[1]-fa[0],fb[1]-fb[0]],
                            "rms_db_delta":(level(&a)-level(&b)).abs(),"distinct_pcm":distinct,"measured_lufs":levels,
                            "measured_true_peak_dbtp":true_peaks,"sample_peaks_dbfs":sample_peaks,
                            "base_silent":a.iter().flatten().all(|v|*v==0.),
                            "candidate_region_peaks":region_peaks,"matches_saved_mix":saved_mix_matches,"pcm_hashes":pcm_hashes}))
                    })()),
                    "window_control" => Some(window_control(&window, &arguments, &mut move_origin)),
                    "file_browse" | "save_export" | "export_name" => {
                        let worker = proxy.clone();
                        let home = root.clone();
                        let operation = op.to_owned();
                        thread::spawn(move || {
                            let result = file_action(&home, &operation, &arguments).map_err(|error| format!("{error:#}"));
                            let _ = worker.send_event(DesktopEvent::Reply(id, result));
                        });
                        None
                    },
                    "call" | "verification_agent_call" if op == "call" || verify => {
                        let worker = proxy.clone();
                        let home = root.clone();
                        let source = if op == "call" { "user" } else { "agent" };
                        thread::spawn(move || {
                            let result = (|| {
                                let name = crate::model::field(&arguments, "name")?;
                                ensure!(source == "user" || matches!(name, "agent_focus" | "project_edit" |
                                    "project_read" | "selection_edit" | "selection_resolve" | "ui_selection_set" | "render_start" |
                                    "range_audition" | "tuning_audition" | "arrangement_audition" |
                                    "audio_clip_edit" | "arrangement_query" | "arrangement_edit" | "desktop_transport"),
                                    "Unsupported Agent verification operation");
                                crate::service::proxy_source(&home, name, arguments.get("arguments").unwrap_or(&json!({})), source)
                            })();
                            let _ = worker.send_event(DesktopEvent::Reply(id, result.map_err(|e|format!("{e:#}"))));
                        });
                        None
                    },
                    _=>Some(Err(anyhow::anyhow!("Unknown desktop operation"))),
                };
                if let Some(result)=immediate{let _=webview.evaluate_script(&reply_script(id,result.map_err(|e|format!("{e:#}"))));}
            },
            _=>{}
        }
    });
}
