'use strict';

const recordButton=document.createElement('button');
recordButton.id='openRecording';
recordButton.title='录音 · R';
recordButton.setAttribute('aria-label','录音');
recordButton.innerHTML=icon('record');
const metronomeButton=document.createElement('button');
metronomeButton.id='metronomeToggle';
metronomeButton.title='节拍器 · M';
metronomeButton.setAttribute('aria-label','节拍器');
metronomeButton.setAttribute('aria-pressed','false');
metronomeButton.innerHTML=icon('metronome');
if(window.desktopNative){
  $('loop').after(metronomeButton,recordButton);
  audio.addEventListener('timeupdate',()=>metronomeButton.setAttribute('aria-pressed',String(audio.clickEnabled)));
  metronomeButton.onclick=async()=>{
    metronomeButton.disabled=true;
    try{await audio.setMetronome(!audio.clickEnabled)}
    catch(error){toast(error.message,true)}
    finally{metronomeButton.disabled=false}
  };
}

const recordDialog=document.createElement('dialog');
recordDialog.className='arrange-dialog record-dialog';
recordDialog.setAttribute('aria-labelledby','recordTitle');
recordDialog.innerHTML=`<div class="dialog-heading"><h2 id="recordTitle">录音</h2><button id="recordClose" aria-label="关闭">${icon('close')}</button></div>
  <div id="recordInputFields" class="record-fields">
    <label>输入设备<select id="recordDevice"></select></label>
    <label>通道<select id="recordChannels"></select></label>
  </div>
  <div id="recordSyncMode" class="record-sync">
    <label><input type="checkbox" id="recordOverdub"> 随伴奏录音</label>
    <label><input type="checkbox" id="recordClick"> 节拍提示</label>
    <div id="recordSyncFields" class="record-fields" hidden>
      <label>预备 · 四分音符拍<input id="recordLead" type="number" min="0" max="8" value="2" step="any"></label>
      <label>延迟校正 · ms<input id="recordCorrection" type="number" min="-500" max="500" value="0" step="any"></label>
    </div>
  </div>
  <div id="recordPlacement" class="record-fields">
    <label>目标音轨<select id="recordTrack"></select></label>
    <label>位置 · 拍<input id="recordBeat" type="number" min="0" max="50000" step="any"></label>
  </div>
  <div class="record-level"><meter id="recordPeak" aria-label="输入采样峰值" min="0" max="1"></meter><output id="recordClock"></output></div>
  <div class="arrange-dialog-footer"><output id="recordStatus" role="status"></output>
    <button id="recordDiscard" class="ghost" hidden>丢弃</button>
    <button id="recordAction" class="primary">开始录音</button></div>`;
document.body.append(recordDialog);
wireDialog(recordDialog);

let recordingState={phase:'idle'},recordingDevices=[],recordBusy=false,recordTimer=0,recordProject=null;
const captureCall=(action,args={})=>window.desktopNative.invoke('recording',{action,...args});

function updateRecordChannels(){
  const device=recordingDevices.find(d=>String(d.device_id)===$('recordDevice').value);
  const options=[];
  for(let i=1;i<=(device?.channels||0);i++){
    options.push(new Option('单声道 '+i,i+'/1'));
    if(i<device.channels)options.push(new Option('立体声 '+i+'–'+(i+1),i+'/2'));
  }
  $('recordChannels').replaceChildren(...options);
}

function updateRecordPlacement(track,start){
  $('recordTrack').replaceChildren(new Option('新建音频轨',''),...project.tracks.filter(isAudioTrack).map(t=>new Option(t.name,t.id)));
  if([...$('recordTrack').options].some(option=>option.value===track))$('recordTrack').value=track;
  $('recordBeat').value=start;
}

function paintRecording(){
  const state=recordingState,active=state.phase==='recording',ready=state.phase==='ready';
  const sameProject=!!project&&(!state.project_id||state.project_id===project.id)&&project.id===recordProject;
  recordButton.classList.toggle('is-recording',active);
  recordButton.title=active?'录音中':ready?'待导入录音 · R':'录音 · R';
  recordButton.setAttribute('aria-label',recordButton.title);
  $('recordInputFields').hidden=active||ready;
  $('recordSyncMode').hidden=active||ready;
  $('recordSyncFields').hidden=!$('recordOverdub').checked&&!$('recordClick').checked;
  $('recordPlacement').hidden=active;
  $('recordTrack').disabled=$('recordBeat').disabled=recordBusy||!sameProject;
  $('recordClose').disabled=active||recordBusy;
  $('recordDiscard').hidden=!ready;
  $('recordDiscard').disabled=recordBusy;
  $('recordAction').textContent=active?'停止录音':ready?'导入工程':'开始录音';
  $('recordAction').disabled=recordBusy||(!active&&(!sameProject||!ready&&(!recordingDevices.length||!audio.paused)));
  $('recordPeak').value=Math.min(1,state.peak||0);
  const seconds=state.seconds||0,minutes=Math.floor(seconds/60);
  $('recordClock').textContent=String(minutes).padStart(2,'0')+':'+(seconds%60).toFixed(2).padStart(5,'0');
  $('recordStatus').textContent=state.error||state.warning||
    (active?(state.preroll?'预备拍 · 等待入点':state.device):ready&&!sameProject?'打开原工程后可导入':
      ready?(state.aligned?'已按入点对齐 · 待导入':state.device+' · 待导入'):!audio.paused?'停止播放后开始录音':
      recordingDevices.length?'':'没有可用输入设备');
}

function pollRecording(){
  clearTimeout(recordTimer);
  if(recordingState.phase!=='recording'||!recordDialog.open)return;
  recordTimer=setTimeout(async()=>{
    if(recordBusy){pollRecording();return}
    try{
      const current=await captureCall('status');
      if(recordBusy)return;
      const wasActive=recordingState.phase==='recording';
      recordingState=current;
      if(wasActive&&current.phase==='ready')updateRecordPlacement(current.track_id,current.start);
      paintRecording();
    }catch(error){$('recordStatus').textContent=error.message}
    pollRecording();
  },100);
}

recordButton.onclick=async()=>{
  if(!project||recordDialog.open||recordBusy)return;
  recordProject=project.id;
  recordBusy=true;
  recordDialog.showModal();
  paintRecording();
  let failure;
  try{
    recordingState=await captureCall('status');
    if(recordingState.phase==='idle'){
      recordingDevices=await captureCall('devices');
      $('recordDevice').replaceChildren(...recordingDevices.map(d=>new Option(d.name+' · '+d.sample_rate+' Hz',d.device_id)));
      const fallback=recordingDevices.find(d=>d.default);
      if(fallback)$('recordDevice').value=fallback.device_id;
      updateRecordChannels();
    }
    updateRecordPlacement(recordingState.track_id??(isAudioTrack(tr())?trackId:''),recordingState.start??cursorBeat);
  }catch(error){failure=error.message}
  finally{recordBusy=false;paintRecording();pollRecording()}
  if(failure)$('recordStatus').textContent=failure;
};

$('recordDevice').onchange=updateRecordChannels;
$('recordOverdub').onchange=paintRecording;
$('recordClick').onchange=paintRecording;
$('recordClose').onclick=()=>recordDialog.close();
recordDialog.addEventListener('cancel',event=>{
  if(recordBusy||recordingState.phase==='recording'){
    event.preventDefault();
    event.stopImmediatePropagation();
  }
},{capture:true});
$('recordAction').onclick=async()=>{
  if(recordBusy)return;
  recordBusy=true;paintRecording();
  let failure;
  try{
    if(recordingState.phase==='recording'){
      recordingState=await captureCall('stop');
      if(recordingState.phase==='ready')updateRecordPlacement(recordingState.track_id,recordingState.start);
    }else{
      const start=$('recordBeat').valueAsNumber,target=$('recordTrack').value||null;
      if(!Number.isFinite(start)||start<0||start>50000)throw new Error('请输入有效拍位');
      await editChain;
      if(project?.id!==recordProject)throw new Error('工程已切换，请重新打开录音');
      const binding={project_id:project.id,expected_revision:project.revision,start,track_id:target};
      if(recordingState.phase==='ready'){
        const take=recordingState.id;
        const result=await captureCall('commit',{...binding,take_id:take,name:'录音'});
        recordingState=result.recording;
        if(project?.id===recordProject){
          trackId=target||result.project.tracks.at(-1).id;
          accept(result.project);
          const track=tr(),clip=trackClips(track).find(c=>c.asset==='audio_'+take+'.wav');
          if(clip)selectAudioClip(track,clip);
          await refreshList();
        }
        recordDialog.close();
      }else{
        if(!audio.paused)throw new Error('请先停止播放');
        const [first_channel,channels]=$('recordChannels').value.split('/').map(Number);
        let overdub;
        const withBacking=$('recordOverdub').checked,click=$('recordClick').checked;
        if(withBacking||click){
          const lead_beats=$('recordLead').valueAsNumber,correction_ms=$('recordCorrection').valueAsNumber;
          if(!Number.isFinite(lead_beats)||!$('recordLead').validity.valid||
            !Number.isFinite(correction_ms)||!$('recordCorrection').validity.valid)throw new Error('请输入有效的预听拍数和校正毫秒数');
          overdub={click,lead_beats,correction_ms};
          if(withBacking){
            $('recordStatus').textContent='正在准备伴奏…';
            await render(false);
            if(!rendered||rendered.project_id!==binding.project_id||rendered.revision!==binding.expected_revision||
              project?.id!==binding.project_id||project.revision!==binding.expected_revision)throw new Error('需要当前工程版本的伴奏混音');
            overdub.job_id=rendered.job_id;
          }
        }
        recordingState=await captureCall('start',{binding,device_id:Number($('recordDevice').value),first_channel,channels,overdub});
      }
    }
  }catch(error){
    failure=error.message;
    if(error.message.includes('REVISION_CONFLICT')&&project?.id===recordProject){
      await load(recordProject).catch(()=>{});
      failure='工程已更新，确认目标音轨和拍位后再次应用';
    }
  }finally{recordBusy=false;paintRecording();pollRecording()}
  if(failure)$('recordStatus').textContent=failure;
};
$('recordDiscard').onclick=async()=>{
  if(recordBusy||recordingState.phase!=='ready')return;
  if(!await studioConfirm('丢弃这段录音？','待导入录音将被删除。','丢弃'))return;
  try{recordingState=await captureCall('discard');recordDialog.close();paintRecording()}
  catch(error){$('recordStatus').textContent=error.message}
};
if(window.desktopNative){
  void captureCall('status').then(state=>{recordingState=state;paintRecording()}).catch(()=>{});
  for(const event of ['play','pause','ended'])audio.addEventListener(event,()=>{if(recordDialog.open)paintRecording()});
}
