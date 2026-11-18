use crate::{
    live_mix::{Advance, Ready, StartPreparation, Work},
    model::{field, Project},
    transport::{loop_bounds, seconds, Transport},
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering::SeqCst},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

const ENDPOINT: &str = "runtime/transport-endpoint.json";
const ROUTE: &str = "/transport";
const SECRET_HEADER: &str = "X-Studio-Desktop-Token";
const DEADLINE: Duration = Duration::from_secs(12);

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AgentCommand {
    Status,
    Devices,
    Play {
        ticket: String,
        project_id: String,
        revision: u64,
        url: String,
        seconds: [f64; 2],
        repeat: bool,
    },
    Pause {
        ticket: String,
    },
    Stop {
        ticket: String,
    },
    AuditionSide {
        ticket: String,
        id: String,
        project_id: String,
        revision: u64,
        variant: usize,
    },
    ForgetRange {
        id: String,
    },
}
impl AgentCommand {
    pub(crate) fn read_only(&self) -> bool {
        matches!(self, Self::Status | Self::Devices)
    }
}

pub(crate) enum Request {
    User(u64, Value),
    MonitorReady {
        control: Arc<crate::jobs::Control>,
        result: Result<Ready>,
    },
    Capture {
        command: CaptureCommand,
        reply: mpsc::SyncSender<Result<Value>>,
    },
    Agent {
        command: AgentCommand,
        expires: Instant,
        reply: mpsc::SyncSender<Result<Value>>,
    },
}

pub(crate) enum CaptureCommand {
    Prepare {
        id: String,
        project_id: String,
        revision: u64,
        job_id: Option<String>,
        beat: f64,
        lead_beats: f64,
        click: bool,
        clock: Arc<crate::capture_clock::CaptureClock>,
    },
    Start(String),
    End(String),
}

struct CaptureDrive {
    id: String,
    clock: Arc<crate::capture_clock::CaptureClock>,
    lead_seconds: f64,
    punch_seconds: f64,
    started: bool,
    project: (String, u64),
}

#[derive(Deserialize, Serialize)]
struct Endpoint {
    port: u16,
    secret: String,
}

pub(crate) struct Link(PathBuf);

impl Link {
    pub(crate) fn unpublish(&self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.unpublish();
    }
}

pub(crate) fn listen(root: &Path, queue: mpsc::SyncSender<Request>) -> Result<Link> {
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|error| anyhow::anyhow!("Desktop transport listener: {error}"))?;
    let address = server
        .server_addr()
        .to_ip()
        .context("Missing local transport address")?;
    let endpoint = Endpoint {
        port: address.port(),
        secret: crate::model::uid("transport"),
    };
    let path = root.join(ENDPOINT);
    fs::write(&path, serde_json::to_vec(&endpoint)?)?;
    let link = Link(path);
    thread::Builder::new()
        .name("desktop-transport-link".into())
        .spawn(move || {
            let host = address.to_string();
            for mut request in server.incoming_requests() {
                let authorized = request.method() == &tiny_http::Method::Post
                    && request.url() == ROUTE
                    && request
                        .body_length()
                        .is_some_and(|length| (1..=16_384).contains(&length))
                    && request
                        .headers()
                        .iter()
                        .any(|h| h.field.equiv("Host") && h.value.as_str() == host)
                    && request.headers().iter().any(|h| {
                        h.field.equiv(SECRET_HEADER) && h.value.as_str() == endpoint.secret
                    })
                    && request.headers().iter().all(|h| !h.field.equiv("Origin"));
                if !authorized {
                    let _ = request.respond(tiny_http::Response::empty(403));
                    continue;
                }
                let result: Result<Value> = (|| {
                    let mut body = Vec::new();
                    request.as_reader().take(16_384).read_to_end(&mut body)?;
                    let command = serde_json::from_slice(&body)?;
                    let (reply, receipt) = mpsc::sync_channel(1);
                    queue
                        .try_send(Request::Agent {
                            command,
                            expires: Instant::now() + DEADLINE,
                            reply,
                        })
                        .map_err(|_| {
                            anyhow::anyhow!(
                                "TRANSPORT_BUSY: desktop is processing another operation"
                            )
                        })?;
                    receipt
                        .recv_timeout(DEADLINE + Duration::from_secs(1))
                        .context("TRANSPORT_OUTCOME_UNKNOWN: read status before retrying")?
                })();
                let envelope = match result {
                    Ok(status) => json!({"ok":true,"data":status}),
                    Err(error) => json!({"ok":false,"error":format!("{error:#}")}),
                };
                let _ = request.respond(crate::service::response(
                    envelope.to_string().into_bytes(),
                    "application/json",
                    200,
                ));
            }
        })?;
    Ok(link)
}

pub(crate) fn send(root: &Path, command: &AgentCommand) -> Result<Value> {
    let bytes = match fs::read(root.join(ENDPOINT)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if command.read_only() {
                return Ok(json!({"attached":false}));
            }
            bail!("DESKTOP_UNAVAILABLE: open the Studio desktop window")
        }
        Err(error) => return Err(error.into()),
    };
    let endpoint: Endpoint =
        serde_json::from_slice(&bytes).context("DESKTOP_UNAVAILABLE: invalid desktop endpoint")?;
    let address = format!("http://127.0.0.1:{}{ROUTE}", endpoint.port);
    let response = ureq::post(&address)
        .header(SECRET_HEADER, &endpoint.secret)
        .config()
        .proxy(None)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_millis(500)))
        .timeout_global(Some(DEADLINE + Duration::from_secs(2)))
        .build()
        .send_json(command);
    let mut response = match response {
        Err(ureq::Error::ConnectionFailed) => {
            if command.read_only() {
                return Ok(json!({"attached":false}));
            }
            bail!("DESKTOP_UNAVAILABLE: open the Studio desktop window")
        }
        response => response.context(
            "DESKTOP_UNAVAILABLE_OR_OUTCOME_UNKNOWN: read desktop transport status before retrying",
        )?,
    };
    let envelope: Value = response
        .body_mut()
        .with_config()
        .limit(16_384)
        .read_json()?;
    ensure!(
        envelope["ok"] == true,
        "{}",
        envelope["error"]
            .as_str()
            .unwrap_or("Desktop rejected transport request")
    );
    Ok(envelope["data"].clone())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    User,
    Agent,
}

#[derive(Clone)]
struct MonitorStart {
    project_id: String,
    revision: u64,
    engine: String,
    position: f64,
    bounds: Option<(f64, f64)>,
}

pub(crate) struct Controller {
    transport: Transport,
    session: String,
    epoch: Arc<AtomicU64>,
    project: Option<(String, u64)>,
    owner: Option<Owner>,
    preview_seconds: Option<[f64; 2]>,
    user_reserved: bool,
    monitor: Option<crate::live_mix::Session>,
    monitor_refresh: Option<Arc<crate::jobs::Control>>,
    monitor_start: Option<MonitorStart>,
    monitor_error: Option<String>,
    capture: Option<CaptureDrive>,
    click_enabled: bool,
}

fn verify_epoch(epoch: &AtomicU64, expected: u64, expires: Instant) -> Result<()> {
    ensure!(Instant::now() < expires, "TRANSPORT_REQUEST_EXPIRED");
    epoch
        .compare_exchange(expected, expected + 1, SeqCst, SeqCst)
        .map_err(|_| {
            anyhow::anyhow!("STALE_TRANSPORT_TICKET: read status after a transport change")
        })?;
    Ok(())
}

impl Controller {
    pub(crate) fn new(root: &Path, epoch: Arc<AtomicU64>) -> Self {
        Self {
            transport: Transport::with_output_settings(root),
            session: crate::model::uid("desktop"),
            epoch,
            project: None,
            owner: None,
            preview_seconds: None,
            user_reserved: false,
            monitor: None,
            monitor_refresh: None,
            monitor_start: None,
            monitor_error: None,
            capture: None,
            click_enabled: false,
        }
    }

    fn ticket(&self, epoch: u64) -> String {
        format!("{}:{epoch}", self.session)
    }

    fn report(&self, mut status: Value) -> Value {
        let epoch = self.epoch.load(SeqCst);
        status["attached"] = json!(true);
        status["epoch"] = json!(epoch);
        status["ticket"] = json!(self.ticket(epoch));
        status["owner"] = json!(self.owner.map(|owner| match owner {
            Owner::User => "user",
            Owner::Agent => "agent",
        }));
        status["user_reserved"] =
            json!(self.user_reserved || self.monitor_start.is_some() || self.capture.is_some());
        status["capture_reserved"] = json!(self.capture.is_some());
        status["preview_seconds"] = json!(self
            .preview_seconds
            .filter(|_| self.owner == Some(Owner::Agent)));
        status["project_id"] = json!(self.project.as_ref().map(|(id, _)| id));
        status["revision"] = json!(self.project.as_ref().map(|(_, revision)| revision));
        status["monitor_engine"] = json!(self.monitor.as_ref().map(|s| &s.engine));
        status["monitor_track_ids"] = json!(self.monitor.as_ref().map(|s| s.track_ids()));
        status["monitor_revision"] = json!(self.monitor.as_ref().map(|s| s.revision()));
        status["monitor_local_frames"] = json!(self.monitor.as_ref().map(|s| s.local_frames()));
        status["monitor_refreshing"] = json!(self.monitor_refresh.is_some());
        status["monitor_starting"] = json!(self.monitor_start.is_some());
        status["monitor_error"] = json!(self.monitor_error);
        status["click_enabled"] = json!(self.click_enabled);
        status
    }

    fn update_editor_click(&mut self, root: &Path) -> Result<()> {
        let click = match &self.project {
            Some((id, _)) if self.click_enabled => {
                let project = crate::model::Store::new(root.to_path_buf())?.read(id)?;
                let clock = project.timeline();
                Some(crate::metronome::Click::new(
                    clock.pulses([0., clock.beat(912.)]),
                ))
            }
            _ => None,
        };
        self.transport.set_click(click);
        Ok(())
    }

    pub(crate) fn user(
        &mut self,
        root: &Path,
        args: &Value,
        resolve_mix: impl FnOnce(&str) -> Result<PathBuf>,
        prepare: impl FnOnce(Work) -> Result<()>,
    ) -> Result<Value> {
        let command = field(args, "command")?;
        if command == "output_devices" {
            return self.transport.output_devices();
        }
        if self.capture.is_some() && command != "context" {
            ensure!(
                matches!(command, "status" | "loop_sync"),
                "录音期间由录音面板控制伴奏"
            );
            let status = self
                .transport
                .execute(&json!({"command":"status"}), |_| unreachable!())?;
            return Ok(self.report(status));
        }
        if self.monitor_start.is_some()
            && matches!(
                command,
                "stop" | "pause" | "clear" | "play" | "audition_side"
            )
        {
            self.cancel_monitor_refresh();
        }
        let mut native = args.clone();
        let mut refresh_click = command == "clear" && self.click_enabled;
        match command {
            "status" => (),
            "output_device" => {
                ensure!(
                    !self.transport.is_playing()
                        && !self.user_reserved
                        && self.monitor_start.is_none(),
                    "停止播放后再更改输出设备"
                );
                let choice = serde_json::from_value(
                    args.get("device_id").context("请选择输出设备")?.clone(),
                )?;
                self.transport.set_output_preference(root, choice)?;
                self.owner = None;
                self.preview_seconds = None;
                self.monitor_error = None;
                refresh_click = self.click_enabled;
                native["command"] = json!("clear");
            }
            "click" => {
                self.click_enabled = args["enabled"].as_bool().context("请选择是否启用节拍器")?;
                self.update_editor_click(root)?;
                native["command"] = json!("status");
            }
            "audition_side" => {
                let id = field(args, "audition_id")?;
                let variant = match args["variant"].as_str() {
                    Some("A") => 0,
                    Some("B") => 1,
                    _ => bail!("选择 A 或 B 试听"),
                };
                let (manifest, paths) = crate::audition::read(root, id)?;
                ensure!(
                    self.project
                        .as_ref()
                        .is_some_and(|(p, r)| p == &manifest.project_id && *r == manifest.revision),
                    "工程版本已更新，请重新准备试听"
                );
                if !self.transport.has_audition(Some(id)) {
                    let project =
                        crate::model::Store::new(root.to_path_buf())?.read(&manifest.project_id)?;
                    ensure!(
                        project.revision == manifest.revision,
                        "工程版本已更新，请重新准备试听"
                    );
                }
                self.transport
                    .play_audition(manifest, paths, variant, || Ok(()))?;
                self.owner = Some(Owner::User);
                self.monitor = None;
                native["command"] = json!("status");
            }
            "audition_stop" => {
                let id = field(args, "audition_id")?;
                native["command"] = json!(if self.transport.has_audition(Some(id)) {
                    self.owner = None;
                    "clear"
                } else {
                    "status"
                });
            }
            "play_monitor" => {
                let id = field(args, "project_id")?;
                let revision = args["revision"]
                    .as_u64()
                    .context("Missing editor revision")?;
                ensure!(
                    self.project
                        .as_ref()
                        .is_some_and(|(p, r)| p == id && *r == revision),
                    "试听工程版本已变化"
                );
                let project = crate::model::Store::new(root.to_path_buf())?.read(id)?;
                ensure!(project.revision == revision, "试听工程版本已变化");
                let start = MonitorStart {
                    project_id: id.to_owned(),
                    revision,
                    engine: field(args, "engine")?.to_owned(),
                    position: seconds(&args["seconds"])?,
                    bounds: loop_bounds(&args["loop"])?,
                };
                if !self.prepare_monitor_start(project, start, prepare)? {
                    let status = self
                        .transport
                        .execute(&json!({"command":"status"}), |_| unreachable!())?;
                    let mut status = self.report(status);
                    status["monitor_fallback"] = json!(true);
                    return Ok(status);
                }
                native["command"] = json!("status");
            }
            "monitor_mix" => {
                ensure!(self.owner == Some(Owner::User), "请先从编辑器播放当前版本");
                let id = field(args, "project_id")?;
                let revision = args["revision"]
                    .as_u64()
                    .context("Missing editor revision")?;
                ensure!(
                    self.project
                        .as_ref()
                        .is_some_and(|(p, r)| p == id && *r == revision),
                    "试听工程版本已变化"
                );
                let rate = self.transport.output_rate();
                if let Some(session) = &mut self.monitor {
                    ensure!(session.revision() == revision, "正在更新试听音轨");
                    let settings = session.draft(&args["draft"])?;
                    if let Some(update) = session.update(settings, rate)? {
                        self.transport.update_live(update)?;
                    }
                } else {
                    let job = field(args, "job_id").context("请先从当前版本的离线混音开始播放")?;
                    let status = crate::jobs::read(root, job)?;
                    let render = &status["result"];
                    ensure!(
                        status["status"] == "complete"
                            && render["mix_url"].as_str() == self.transport.source(),
                        "试听需要已经载入的混音"
                    );
                    let project = crate::model::Store::new(root.to_path_buf())?.read(id)?;
                    ensure!(project.revision == revision, "试听工程版本已变化");
                    let origin = self.transport.monitor_origin();
                    let (mut session, mixer) =
                        crate::live_mix::Session::load(root, project, render, rate, origin)?;
                    let settings = session.draft(&args["draft"])?;
                    let update = session.update(settings, rate)?;
                    self.transport.begin_live(mixer)?;
                    if let Some(update) = update {
                        self.transport.update_live(update)?;
                    }
                    self.monitor = Some(session);
                    self.monitor_error = None;
                }
                native["command"] = json!("status");
            }
            "context" => {
                let id = field(args, "project_id")?;
                crate::model::identifier(id)?;
                let revision = args["revision"]
                    .as_u64()
                    .context("Missing editor revision")?;
                let changed = !self
                    .project
                    .as_ref()
                    .is_some_and(|(p, r)| p == id && *r == revision);
                self.project = Some((id.to_owned(), revision));
                refresh_click = changed && self.click_enabled && self.capture.is_none();
                let mut current = "status";
                let pending = self
                    .monitor_start
                    .clone()
                    .filter(|start| start.project_id == id);
                if changed {
                    self.cancel_monitor_refresh();
                    self.monitor_error = None;
                }
                if let Some(mut start) = pending.filter(|_| changed) {
                    let project = crate::model::Store::new(root.to_path_buf())?.read(id)?;
                    ensure!(project.revision == revision, "试听工程版本已变化");
                    start.revision = revision;
                    if !self.prepare_monitor_start(project, start, prepare)? {
                        current = "clear";
                    }
                } else if self.owner == Some(Owner::Agent)
                    || changed && self.transport.has_audition(None)
                {
                    self.owner = None;
                    current = "clear";
                } else if let Some(session) = &mut self.monitor {
                    if changed {
                        let next = (|| {
                            let project = crate::model::Store::new(root.to_path_buf())?.read(id)?;
                            ensure!(project.revision == revision, "监听工程版本已更新");
                            session.advance(project, self.transport.monitor_origin())
                        })();
                        match next {
                            Ok(Advance::Settings(settings)) => {
                                if let Some(update) =
                                    session.update(settings, self.transport.output_rate())?
                                {
                                    self.transport.update_live(update)?;
                                }
                            }
                            Ok(Advance::Preparing(work)) => {
                                let control = work.control.clone();
                                if prepare(Work::Update(work)).is_ok() {
                                    self.monitor_refresh = Some(control);
                                } else {
                                    current = "clear";
                                }
                            }
                            Ok(Advance::Incompatible) | Err(_) => current = "clear",
                        }
                    }
                }
                native["command"] = json!(current);
            }
            "reserve" | "release" => {
                self.user_reserved = command == "reserve";
                native["command"] =
                    json!(if self.user_reserved && self.owner == Some(Owner::Agent) {
                        self.owner = Some(Owner::User);
                        "stop"
                    } else {
                        "status"
                    });
            }
            "loop" => {
                if let Some(start) = &mut self.monitor_start {
                    start.bounds = loop_bounds(&args["loop"])?;
                }
                self.owner = Some(Owner::User);
            }
            "loop_sync" => {
                native["command"] = json!(if self.owner == Some(Owner::User)
                    && !self.transport.has_audition(None)
                {
                    "loop"
                } else {
                    "status"
                });
            }
            _ => {
                if self.owner == Some(Owner::Agent) && command == "seek" {
                    // A human seek returns to the editor's regular selection loop, if any.
                    self.transport.execute(
                        &json!({"command":"loop","loop":args["loop"]}),
                        |_| unreachable!(),
                    )?;
                }
                self.owner = if command == "clear" {
                    None
                } else {
                    Some(Owner::User)
                };
            }
        }
        let mut status = self.transport.execute(&native, resolve_mix)?;
        if refresh_click {
            self.update_editor_click(root)?;
            status = self
                .transport
                .execute(&json!({"command":"status"}), |_| unreachable!())?;
        }
        if status["live_mix"] != true {
            self.monitor = None;
            if self.monitor_start.is_none() {
                self.cancel_monitor_refresh();
            }
        }
        Ok(self.report(status))
    }

    fn prepare_monitor_start(
        &mut self,
        project: Project,
        start: MonitorStart,
        prepare: impl FnOnce(Work) -> Result<()>,
    ) -> Result<bool> {
        let rate = self.transport.prepare_live_output()?;
        let origin = start.bounds.map_or(start.position, |(first, end)| {
            if (first..end).contains(&start.position) {
                start.position
            } else {
                first
            }
        });
        let frame = |position: f64| (position * 48_000.).round() as usize;
        let frames = crate::source_cache::frame_count(&project);
        let origin = frame(origin);
        let origin = if origin < frames { origin } else { 0 };
        let Some(work) = StartPreparation::new(
            project,
            &start.engine,
            rate,
            (origin, start.bounds.map(|(a, b)| (frame(a), frame(b)))),
        )?
        else {
            return Ok(false);
        };
        self.cancel_monitor_refresh();
        let work = Work::Start(Box::new(work));
        let control = work.control().clone();
        prepare(work)?;
        self.monitor_refresh = Some(control);
        self.monitor_start = Some(start);
        self.monitor_error = None;
        self.owner = Some(Owner::User);
        self.preview_seconds = None;
        Ok(true)
    }

    fn cancel_monitor_refresh(&mut self) {
        self.monitor_start = None;
        if let Some(control) = self.monitor_refresh.take() {
            let _ = control.cancel();
        }
    }

    pub(crate) fn monitor_ready(
        &mut self,
        root: &Path,
        control: &Arc<crate::jobs::Control>,
        result: Result<Ready>,
    ) -> Result<Option<Value>> {
        if !self
            .monitor_refresh
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(control, pending))
        {
            return Ok(None);
        }
        self.monitor_refresh = None;
        let start = self.monitor_start.take();
        let applied = result.and_then(|ready| match ready {
            Ready::Start(prepared) => {
                let start = start.context("试听请求已经结束")?;
                let project =
                    crate::model::Store::new(root.to_path_buf())?.read(&start.project_id)?;
                ensure!(project.revision == start.revision, "试听工程版本已变化");
                let (session, mixer) = *prepared;
                self.transport
                    .play_live(mixer, start.position, start.bounds)?;
                self.monitor = Some(session);
                Ok(())
            }
            Ready::Update(prepared) => {
                let session = self.monitor.as_mut().context("监听会话已经结束")?;
                let (replacement, update) =
                    session.install(*prepared, self.transport.output_rate())?;
                self.transport.replace_live_sources(replacement, update)
            }
        });
        if let Err(error) = applied {
            self.monitor_error = Some(error.to_string());
            self.monitor = None;
            self.transport
                .execute(&json!({"command":"clear"}), |_| unreachable!())?;
        }
        let status = self
            .transport
            .execute(&json!({"command":"status"}), |_| unreachable!())?;
        Ok(Some(self.report(status)))
    }

    pub(crate) fn capture(
        &mut self,
        root: &Path,
        command: CaptureCommand,
        resolve_mix: impl FnOnce(&str) -> Result<PathBuf>,
    ) -> Result<Value> {
        match command {
            CaptureCommand::Prepare {
                id,
                project_id,
                revision,
                job_id,
                beat,
                lead_beats,
                click,
                clock,
            } => {
                ensure!(
                    self.capture.is_none()
                        && self.monitor_start.is_none()
                        && !self.user_reserved
                        && !self.transport.is_playing(),
                    "当前播放或录音尚未停止"
                );
                ensure!(
                    self.project
                        .as_ref()
                        .is_some_and(|(current, r)| current == &project_id && *r == revision),
                    "录音需要当前工程版本"
                );
                ensure!(
                    (0.0..=8.).contains(&lead_beats) && (0.0..=50_000.).contains(&beat),
                    "无效的伴奏起拍"
                );
                let project = crate::model::Store::new(root.to_path_buf())?.read(&project_id)?;
                ensure!(
                    project.revision == revision,
                    "REVISION_CONFLICT: 重新准备伴奏"
                );
                let timeline = project.timeline();
                let lead = if click {
                    beat - lead_beats
                } else {
                    (beat - lead_beats).max(0.)
                };
                let lead_seconds = timeline.seconds_with_preroll(lead);
                let punch_seconds = timeline.seconds(beat);
                if let Some(job_id) = job_id {
                    let status = crate::jobs::read(root, &job_id)?;
                    let render = &status["result"];
                    ensure!(
                        status["status"] == "complete"
                            && render["project_id"] == project_id
                            && render["revision"] == revision,
                        "伴奏必须为当前版本渲染完成的混音"
                    );
                    let url = field(render, "mix_url")?;
                    self.transport
                        .prepare_capture(Some((url, &resolve_mix(url)?)), punch_seconds)?;
                } else {
                    ensure!(click, "请选择伴奏或节拍器");
                    self.transport.prepare_capture(None, punch_seconds)?;
                }
                self.transport.set_click(click.then(|| {
                    crate::metronome::Click::new(timeline.pulses([lead, timeline.beat(912.)]))
                }));
                self.capture = Some(CaptureDrive {
                    id,
                    clock,
                    lead_seconds,
                    punch_seconds,
                    started: false,
                    project: (project_id, revision),
                });
                self.owner = Some(Owner::User);
                self.monitor = None;
                self.cancel_monitor_refresh();
                self.epoch.fetch_add(1, SeqCst);
            }
            CaptureCommand::Start(id) => {
                let capture = self
                    .capture
                    .as_mut()
                    .filter(|owner| owner.id == id && !owner.started)
                    .context("录音伴奏已失效")?;
                ensure!(
                    self.project.as_ref() == Some(&capture.project)
                        && crate::model::Store::new(root.to_path_buf())?
                            .read(&capture.project.0)?
                            .revision
                            == capture.project.1,
                    "REVISION_CONFLICT: 工程已更新，请重新准备伴奏"
                );
                self.transport.start_capture(
                    capture.clock.clone(),
                    capture.lead_seconds,
                    capture.punch_seconds,
                );
                capture.started = true;
            }
            CaptureCommand::End(id) => {
                if self.capture.as_ref().is_some_and(|owner| owner.id == id) {
                    self.transport.end_capture();
                    self.capture = None;
                    self.epoch.fetch_add(1, SeqCst);
                    self.update_editor_click(root)?;
                }
            }
        }
        let status = self
            .transport
            .execute(&json!({"command":"status"}), |_| unreachable!())?;
        Ok(self.report(status))
    }

    pub(crate) fn agent(
        &mut self,
        root: &Path,
        command: AgentCommand,
        expires: Instant,
        resolve_mix: impl FnOnce(&str) -> Result<PathBuf>,
    ) -> Result<Value> {
        let snapshot = |transport: &mut Transport| {
            transport.execute(&json!({"command":"status"}), |_| unreachable!())
        };
        if matches!(command, AgentCommand::Status) {
            let status = snapshot(&mut self.transport)?;
            return Ok(self.report(status));
        }
        if matches!(command, AgentCommand::Devices) {
            return self.transport.output_devices();
        }
        if let AgentCommand::ForgetRange { id } = &command {
            ensure!(
                id.starts_with("ranges_")
                    || id.starts_with("pitch_ab_")
                    || id.starts_with("agent_tuning_")
                    || id.starts_with("arrange_ab_")
                    || id.starts_with("agent_arrange_"),
                "Unknown range or tuning audition"
            );
            if self.transport.has_audition(Some(id)) {
                self.epoch.fetch_add(1, SeqCst);
                self.owner = None;
                self.preview_seconds = None;
                self.transport
                    .execute(&json!({"command":"clear"}), |_| unreachable!())?;
            }
            let status = snapshot(&mut self.transport)?;
            return Ok(self.report(status));
        }
        let ticket = match &command {
            AgentCommand::Play { ticket, .. }
            | AgentCommand::AuditionSide { ticket, .. }
            | AgentCommand::Pause { ticket }
            | AgentCommand::Stop { ticket } => ticket,
            AgentCommand::Status | AgentCommand::Devices | AgentCommand::ForgetRange { .. } => {
                unreachable!()
            }
        };
        let epoch = self.epoch.load(SeqCst);
        ensure!(
            *ticket == self.ticket(epoch),
            "STALE_TRANSPORT_TICKET: read status after a transport change"
        );
        ensure!(Instant::now() < expires, "TRANSPORT_REQUEST_EXPIRED");
        let commit = || verify_epoch(&self.epoch, epoch, expires);
        let status = match command {
            AgentCommand::Play {
                project_id,
                revision,
                url,
                seconds,
                repeat,
                ..
            } => {
                ensure!(self.project.as_ref().is_some_and(|(id, rev)| *id == project_id && *rev == revision), "DESKTOP_PROJECT_MISMATCH: the requested revision must be open in the desktop editor");
                ensure!(
                    !(self.user_reserved
                        || self.monitor_start.is_some()
                        || self.capture.is_some()
                        || self.owner == Some(Owner::User) && self.transport.is_playing()),
                    "USER_TRANSPORT_ACTIVE: playback is under user control"
                );
                self.transport.play(
                    &url,
                    seconds[0],
                    Some((seconds[0], seconds[1])),
                    repeat,
                    resolve_mix,
                    commit,
                )?;
                self.monitor = None;
                self.cancel_monitor_refresh();
                self.owner = Some(Owner::Agent);
                self.preview_seconds = Some(seconds);
                snapshot(&mut self.transport)?
            }
            AgentCommand::Pause { .. } | AgentCommand::Stop { .. } => {
                ensure!(
                    self.owner == Some(Owner::Agent),
                    "AGENT_PREVIEW_NOT_OWNER: user controls this playback"
                );
                commit()?;
                let stop = matches!(command, AgentCommand::Stop { .. });
                let clear_pair = stop && self.transport.has_audition(None);
                if stop {
                    self.owner = None;
                }
                self.transport.execute(
                    &json!({"command":if clear_pair {"clear"} else if stop {"stop"} else {"pause"}}),
                    |_| unreachable!(),
                )?
            }
            AgentCommand::AuditionSide {
                id,
                project_id,
                revision,
                variant,
                ..
            } => {
                ensure!(variant < 2, "Choose A or B");
                ensure!(
                    self.project
                        .as_ref()
                        .is_some_and(|(id, rev)| *id == project_id && *rev == revision),
                    "DESKTOP_PROJECT_MISMATCH"
                );
                ensure!(
                    !(self.user_reserved
                        || self.monitor_start.is_some()
                        || self.capture.is_some()
                        || self.owner == Some(Owner::User) && self.transport.is_playing()),
                    "USER_TRANSPORT_ACTIVE: playback is under user control"
                );
                let (manifest, paths) = crate::audition::read(root, &id)?;
                ensure!(
                    (manifest.saved.is_some()
                        || manifest.recordings.is_some()
                        || id.starts_with("agent_tuning_")
                        || id.starts_with("agent_arrange_"))
                        && manifest.project_id == project_id
                        && manifest.revision == revision,
                    "A/B 试听版本已更新"
                );
                let seconds = manifest.ranges()[variant]
                    .map(|beat| manifest.timing[variant].clock().seconds(beat));
                self.transport.play_audition(manifest, paths, variant, || {
                    let current =
                        crate::model::Store::new(root.to_path_buf())?.read(&project_id)?;
                    ensure!(
                        current.revision == revision,
                        "REVISION_CONFLICT: 工程已更新"
                    );
                    commit()
                })?;
                self.monitor = None;
                self.cancel_monitor_refresh();
                self.owner = Some(Owner::Agent);
                self.preview_seconds = Some(seconds);
                snapshot(&mut self.transport)?
            }
            AgentCommand::Status | AgentCommand::Devices | AgentCommand::ForgetRange { .. } => {
                unreachable!()
            }
        };
        Ok(self.report(status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_a_pending_monitor_rejects_its_late_result() -> Result<()> {
        let root = Path::new(".");
        let mut controller = Controller::new(root, Arc::new(AtomicU64::new(0)));
        let control = Arc::new(crate::jobs::Control::ephemeral(
            "test", "project", 1, "guide",
        ));
        controller.monitor_refresh = Some(control.clone());
        controller.monitor_start = Some(MonitorStart {
            project_id: "project".into(),
            revision: 1,
            engine: "guide".into(),
            position: 0.,
            bounds: None,
        });
        let status = controller.user(
            root,
            &json!({"command":"stop"}),
            |_| unreachable!(),
            |_| unreachable!(),
        )?;
        assert_eq!(status["monitor_starting"], false);
        assert_eq!(status["playing"], false);
        assert!(controller
            .monitor_ready(root, &control, Err(anyhow::anyhow!("late result")))?
            .is_none());
        Ok(())
    }

    #[test]
    fn expired_or_superseded_commands_cannot_commit() {
        let epoch = AtomicU64::new(4);
        let deadline = Instant::now() + Duration::from_secs(2);
        assert!(verify_epoch(&epoch, 3, deadline)
            .unwrap_err()
            .to_string()
            .contains("STALE_TRANSPORT_TICKET"));
        assert!(verify_epoch(&epoch, 4, Instant::now()).is_err());
        assert_eq!(epoch.load(SeqCst), 4);
        verify_epoch(&epoch, 4, deadline).unwrap();
        assert_eq!(epoch.load(SeqCst), 5);
    }
}
