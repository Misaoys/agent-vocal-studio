'use strict';

class DesktopAudio extends EventTarget {
  #source = null;
  #generation = 0;
  #timer;
  #loop = '';
  #time = 0;
  #status = null;
  #playing = false;

  constructor() {
    super();
    this.meter = document.createElement('span');
    this.meter.className = 'transport-meter';
    this.meter.setAttribute('aria-hidden', 'true');
    this.meter.append(document.createElement('i'), document.createElement('i'));
    document.getElementById('timeDisplay').after(this.meter);
    window.desktopAudioUpdate = status => this.#accept(status);
  }

  get paused() { return !this.#playing }
  get currentTime() { return this.#time }
  get duration() { return this.#status?.duration || 0 }
  get readyState() { return this.duration > 0 ? 1 : 0 }
  get agentRange() { return this.#status?.preview_seconds }
  get liveMix() { return this.#status?.live_mix === true }
  get liveEngine() { return this.#status?.monitor_engine }
  get trackMeters() { return this.#playing?this.#status?.track_peaks||[]:[] }
  get monitorTrackIds() { return this.#status?.monitor_track_ids||[] }
  get auditionId() { return this.#status?.audition_id }
  get auditionVariant() { return this.#status?.audition_variant }
  get clickEnabled() { return this.#status?.click_enabled===true }

  set currentTime(value) {
    if (!Number.isFinite(value)) return;
    this.#time = Math.min(this.duration, Math.max(0, value));
    this.#emit('timeupdate');
    this.#send('seek', {seconds: this.#time, loop: loopRange()}).catch(error => toast(error.message, true));
  }

  #emit(name) {
    const event = new Event(name);
    this['on' + name]?.(event);
    this.dispatchEvent(event);
  }

  #setPlaying(value, ended = false) {
    if (this.#playing === value) return;
    this.#playing = value;
    if (!value) this.#updateMeter();
    this.#emit(value ? 'play' : ended ? 'ended' : 'pause');
  }

  #updateMeter() {
    const state = this.#status;
    const peaks = this.#playing ? state?.peak || [] : [];
    [...this.meter.children].forEach((bar, index) => {
      const db = 20 * Math.log10(Math.max(peaks[index] || 0, 0.001));
      bar.style.setProperty('--level', Math.max(0, 1 + db / 60));
    });
    this.meter.title = state?.device ? `${state.device} · ${state.output_rate / 1000} kHz` : 'Windows 音频输出';
  }

  #accept(status) {
    if (this.#status && status.epoch < this.#status.epoch) return;
    clearTimeout(this.#timer);
    const newlyInterrupted=status.stream_error&&!this.#status?.stream_error;
    const refreshFailed=status.monitor_error&&status.monitor_error!==this.#status?.monitor_error;
    this.#status = status;
    const play=document.getElementById('play');
    play.setAttribute('aria-busy',String(!!(status.buffering||status.monitor_refreshing)));
    const statusText=document.getElementById('statusText');
    if(status.monitor_refreshing)statusText.textContent='正在更新试听音轨';
    else if(refreshFailed)statusText.textContent='试听更新失败 · '+status.monitor_error;
    else if(statusText.textContent==='正在更新试听音轨')statusText.textContent=status.live_mix?'试听已更新':'';
    if(newlyInterrupted)toast('播放文件读取中断，请重新打开试听',true);
    this.#source = status.url;
    this.#time = status.audition_timeline_seconds??status.seconds;
    this.#setPlaying(status.playing, status.ended);
    this.#updateMeter();
    this.#emit('timeupdate');
    if (status.playing) this.#timer = setTimeout(() => this.#poll(), 32);
  }

  async #send(command, options = {}) {
    const generation = ++this.#generation;
    clearTimeout(this.#timer);
    try {
      const status = await window.desktopNative.invoke('transport', {command, ...options});
      if (generation === this.#generation) this.#accept(status);
      return status;
    } catch (error) {
      if (generation !== this.#generation) return;
      if(command==='monitor_mix'||command==='audition_side'||command==='click'){
        if(this.#playing)this.#timer=setTimeout(()=>this.#poll(),32);
      }else this.#setPlaying(false);
      throw error;
    }
  }

  #poll() {
    if (this.#status?.owner === 'agent'||this.auditionId) {
      this.#send('status').catch(error => toast(error.message, true));
      return;
    }
    const range = loopRange();
    const key = JSON.stringify(range);
    const changed = key !== this.#loop;
    this.#loop = key;
    this.#send(changed ? 'loop_sync' : 'status', changed ? {loop: range} : {})
      .catch(error => toast(error.message, true));
  }

  async playSource(url, cursorSeconds, range) {
    const position = this.#source === url ? this.currentTime : cursorSeconds;
    this.#source = url;
    this.#loop = JSON.stringify(range);
    await this.#send('play', {url, seconds: position, loop: range});
  }

  async setProject(project) {
    await this.#send('context', {project_id: project.id, revision: project.revision});
  }

  async setMetronome(enabled) {
    return this.#send('click', {enabled});
  }

  async selectOutput(device_id) {
    const status=await window.desktopNative.invoke('transport',{command:'output_device',device_id});
    this.#accept(status);
    return status;
  }

  async reservePreview(active) {
    await this.#send(active ? 'reserve' : 'release');
  }

  async monitorMix(project, jobId, draft) {
    return this.#send('monitor_mix', {project_id: project.id, revision: project.revision, job_id: jobId, draft});
  }

  async resumeLive(range) {
    await this.#send('resume_live', {loop:range});
  }

  async auditionSide(audition_id,variant){
    return this.#send('audition_side',{audition_id,variant});
  }

  async stopAudition(audition_id=this.auditionId){
    if(audition_id)await this.#send('audition_stop',{audition_id});
  }

  setUserLoop(range) {
    this.#loop = JSON.stringify(range);
    this.#send('loop', {loop: range}).catch(error => toast(error.message, true));
  }

  pause() {
    this.#setPlaying(false);
    if (this.#source) this.#send('pause').catch(error => toast(error.message, true));
  }

  stop() {
    this.#setPlaying(false);
    this.#time = 0;
    this.#emit('timeupdate');
    this.#send('stop').catch(error => toast(error.message, true));
  }

  removeAttribute(name) {
    if (name !== 'src' || (!this.#source&&!this.auditionId)) return;
    this.#source = null;
    this.#status = null;
    this.#time = 0;
    this.#setPlaying(false);
    this.#emit('timeupdate');
    this.#send('clear').catch(error => toast(error.message, true));
  }
}

function paintAgentPreview(ctx, audio, x, top, height) {
  const range = audio.agentRange;
  if (!range) return;
  const left = x(secondsBeat(range[0])), right = x(secondsBeat(range[1]));
  ctx.fillStyle = '#d9d9d91a';
  ctx.fillRect(left, top, right-left, height);
  ctx.fillStyle = '#ebebeb68';
  ctx.fillRect(left, top, 1, height);
  ctx.fillRect(right, top, 1, height);
}
