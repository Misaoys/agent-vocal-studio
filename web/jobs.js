'use strict';
const renderProgress=document.createElement('progress');renderProgress.id='renderProgress';renderProgress.max=1;renderProgress.hidden=true;renderProgress.setAttribute('aria-label','渲染工作进度');renderProgress.title='已完成工作量，非剩余时间预测';
const cancelRender=document.createElement('button');cancelRender.id='cancelRender';cancelRender.className='ghost';cancelRender.hidden=true;cancelRender.innerHTML=icon('close')+'<span>停止渲染</span>';cancelRender.title='请求停止；歌声引擎当前请求返回后结束';
$('statusText').after(renderProgress,cancelRender);
let activeRenderJob=null,checkingRemoteJob=false;
const jobStageNames={preparing:'准备音源',preparing_voice:'连接歌声引擎',voice_query:'分析歌声',voice_synthesis:'合成歌声',instrument:'渲染乐器',audio:'读取伴奏',sidechain_sources:'准备旁链触发',guide:'生成导唱',cached_source:'复用原始音轨',cached_voice_preview:'复用已试听歌声',track_effects:'处理音轨效果器',mix_buses:'处理编组与返回',master_effects:'计算总线限制',mixing:'混音',writing_stems:'写入分轨',writing_mix:'写入混音',committing:'保存音频'};
function showRenderJob(job){
  const cancelling=job.status==='cancelling';
  const description=(cancelling?'正在停止 · 等待当前处理返回':jobStageNames[job.stage]||'正在渲染')+(job.track_name?' · '+job.track_name:'');
  const terminal={complete:'试听文件已渲染',cancelled:'渲染已取消',failed:'渲染失败',interrupted:'渲染已中断'};
  $('statusText').textContent=terminal[job.status]||description;
  updateAgentRender(job,terminal[job.status]||description);
  renderProgress.value=job.progress||0;renderProgress.setAttribute('aria-valuetext',Math.round((job.progress||0)*100)+'%');
  cancelRender.disabled=!job.can_cancel;cancelRender.querySelector('span').textContent=cancelling?'正在停止':'停止渲染';
}
function renderJobVisibility(visible){
  rendering=visible;$('play').disabled=visible;$('exportWav').disabled=visible;
  for(const el of [renderProgress,cancelRender]){
    if(visible&&el.hidden){el.hidden=false;animateElement(el,[{opacity:0,transform:'translateY(3px)'},{opacity:1,transform:'translateY(0)'}],'insert')}
    else if(!visible&&!el.hidden){const animation=animateElement(el,[{opacity:1},{opacity:0}],'remove');animation.finished.catch(()=>{}).finally(()=>{if(!rendering)el.hidden=true})}
  }
}
cancelRender.onclick=async()=>{if(!activeRenderJob)return;try{cancelRender.disabled=true;const v=await api('render_cancel',{job_id:activeRenderJob});showRenderJob(v.job)}catch(e){toast(e.message,true);cancelRender.disabled=false}};
async function playRenderedAudio(result){
  const url=result.mix_url;
  playingArchivedMix=result.archived_mix===true;
  if(window.desktopNative){
    const binding=project;
    await audio.playSource(url,beatSeconds(cursorBeat),loopRange());
    if(project?.id===binding.id&&project.revision===binding.revision&&result.preview_sources?.length&&
      binding.master_limiter?.peak_mode!=='true_peak'){
      try{await audio.monitorMix(binding,result.job_id,{tracks:[]})}
      catch(error){$('statusText').textContent='混音试听 · '+error.message}
    }
    return;
  }
  const newSource=audio.getAttribute('src')!==url;
  if(newSource)audio.src=url;
  const target=loopTime(newSource?beatSeconds(cursorBeat):audio.currentTime);
  if(target>0&&audio.readyState===0){
    await new Promise((resolve,reject)=>{
      const finish=()=>{
        audio.removeEventListener('loadedmetadata',ready);
        audio.removeEventListener('error',failed);
      };
      const ready=()=>{finish();resolve()};
      const failed=()=>{finish();reject(new Error('音频载入失败'))};
      audio.addEventListener('loadedmetadata',ready,{once:true});
      audio.addEventListener('error',failed,{once:true});
    });
  }
  if(target!==audio.currentTime)audio.currentTime=Math.min(audio.duration,target);
  await audio.play();
}
function currentMixScope(){
  if(!project)throw new Error('请先打开工程');
  return {project_id:project.id,expected_revision:project.revision};
}
function sameMixScope(scope){
  return project?.id===scope.project_id&&project.revision===scope.expected_revision;
}
async function archivedMix(scope){
  const {jobs}=await api('render_jobs',{...scope,archived_only:true});
  return jobs[0]?.result;
}
async function playArchivedMix(){
  if(rendering)throw new Error('请等待当前渲染完成');
  const epoch=previewEpoch;
  await editChain;
  const scope=currentMixScope();
  const mix=await archivedMix(scope);
  if(!sameMixScope(scope)||epoch!==previewEpoch)return;
  if(!mix)throw new Error('当前版本没有归档混音');
  await playRenderedAudio(mix);
  $('statusText').textContent=renderReady(mix,'归档混音');
}
async function saveProjectWithMix(){
  await editChain;
  const scope=currentMixScope();
  let mix=sameMixScope({project_id:rendered?.project_id,expected_revision:rendered?.revision})
    && rendered.audio_verified_non_silent ? rendered : null;
  if(playingArchivedMix)mix=await archivedMix(scope);
  if(!mix){
    const {jobs}=await api('render_jobs',{...scope,engine:$('renderEngine').value});
    mix=jobs[0]?.result;
  }
  if(!mix)mix=await archivedMix(scope);
  if(!sameMixScope(scope))return;
  if(!mix){
    await render(false);
    if(!sameMixScope(scope))return;
    mix=rendered;
  }
  if(!mix?.audio_verified_non_silent)throw new Error('当前版本没有可归档的混音');
  const pack=await api('project_export',{...scope,format:'avpack',mix_job_id:mix.job_id});
  await download(pack.url);
}
async function monitorRenderJob(id,options={}){
  if(activeRenderJob)return;
  activeRenderJob=id;renderJobVisibility(true);
  try{
    for(;;){
      let v;try{v=await api('render_status',{job_id:id})}catch(e){
        if(e.message.includes('Unknown job'))throw e;
        $('statusText').textContent='任务连接中断 · 正在恢复';await new Promise(r=>setTimeout(r,1500));continue;
      }
      showRenderJob(v);
      if(v.status==='cancelled'){$('statusText').textContent='渲染已停止';return}
      if(v.status==='failed'||v.status==='interrupted')throw new Error(v.error||'渲染未完成');
      if(v.status==='complete'){
        const result=v.result;
        if(project?.id!==result.project_id||project?.revision!==result.revision){$('statusText').textContent='音频已保存 · 工程已有更新';return}
        rendered=result;$('statusText').textContent=renderReady(result,'音频就绪');
        if(options.save)showWaveExport(result,options.exportBeats);
        if(options.play&&previewEpoch===options.previewEpoch){
          try{await playRenderedAudio(result)}catch(error){toast(error.message,true)}
        }
        return;
      }
      await new Promise(r=>setTimeout(r,400));
    }
  }catch(e){toast(e.message,true);$('statusText').textContent='渲染未完成 · '+e.message}
  finally{activeRenderJob=null;renderJobVisibility(false)}
}

function renderReady(result,label){
  const {integrated_lufs,true_peak_dbtp}=result.loudness||{};
  const metrics=[];
  if(Number.isFinite(integrated_lufs))metrics.push(integrated_lufs.toFixed(1)+' LUFS');
  if(Number.isFinite(true_peak_dbtp))metrics.push(true_peak_dbtp.toFixed(1)+' dBTP');
  else if(Number.isFinite(result.peak_dbfs))metrics.push(result.peak_dbfs.toFixed(1)+' dBFS');
  return [label,result.seconds.toFixed(1)+' 秒',...metrics].join(' · ');
}

async function playTruePeakMix(){
  if(project?.master_limiter?.peak_mode!=='true_peak')throw new Error('请先在总线限制器启用真峰值');
  await render(true,false,true);
}

render=async function(play=false,save=false,exactPreview=false){
  if(!project||rendering||play&&audio.monitorStarting)return;
  const requestedPreview=previewEpoch,requestedProject=project.id;
  const exportBeats=save&&arrangeSelection?[arrangeSelection.start,arrangeSelection.end]:null;
  const reserve=play&&window.desktopNative;
  rendering=true;$('play').disabled=true;$('exportWav').disabled=true;
  try{
    if(reserve)await audio.reservePreview(true);
    await editChain.catch(()=>{});
    if(project?.id!==requestedProject||play&&previewEpoch!==requestedPreview)return;
    const engine=$('renderEngine').value;
    if(play&&!save&&window.desktopNative&&!exactPreview){
      if(audio.liveMix&&audio.liveEngine===engine){
        try{await audio.resumeLive(loopRange());return}catch{}
      }
      const status=await audio.playMonitor(project,engine,beatSeconds(cursorBeat),loopRange());
      if(!status?.monitor_fallback){
        playingArchivedMix=false;
        return;
      }
    }
    const scope={project_id:project.id,expected_revision:project.revision,engine};
    if(rendered?.revision!==scope.expected_revision||rendered.project_id!==scope.project_id||rendered.engine!==engine){
      const {jobs}=await api('render_jobs',scope);
      if(project?.id!==scope.project_id||project?.revision!==scope.expected_revision)return;
      rendered=jobs[0]?.result||null;
    }
    const hasCurrent=rendered?.revision===project.revision&&rendered.project_id===project.id&&rendered.engine===engine;
    if(hasCurrent&&(play&&!save||rendered.loudness)&&(!save||rendered.audio_verified_non_silent)){
      $('statusText').textContent=renderReady(rendered,'试听就绪');
      if(save)showWaveExport(rendered,exportBeats);
      if(play&&previewEpoch===requestedPreview)await playRenderedAudio(rendered);
      return;
    }
    const job=await api('render_start',scope);
    await monitorRenderJob(job.job_id,{play,save,exportBeats,previewEpoch:requestedPreview});
  }catch(e){toast(e.message,true)}
  finally{
    if(reserve)await audio.reservePreview(false).catch(error=>toast(error.message,true));
    if(!activeRenderJob)renderJobVisibility(false);
  }
};
async function findActiveRenderJob(){
  if(activeRenderJob||rendering||checkingRemoteJob)return;checkingRemoteJob=true;
  try{const list=await api('render_jobs',{active_only:true});if(activeRenderJob||rendering)return;const job=list.jobs.find(j=>['running','cancelling','committing'].includes(j.status));if(job)void monitorRenderJob(job.job_id)}catch{}finally{checkingRemoteJob=false}
}
setTimeout(findActiveRenderJob,200);setInterval(findActiveRenderJob,2000);

const waveExportDialog=document.createElement('dialog');
waveExportDialog.id='waveExportDialog';
waveExportDialog.className='arrange-dialog';
waveExportDialog.setAttribute('aria-labelledby','waveExportTitle');
waveExportDialog.innerHTML=`
  <div class="dialog-heading"><h2 id="waveExportTitle">导出 WAV</h2><button id="waveExportClose" aria-label="关闭">${icon('close')}</button></div>
  <form id="waveExportForm">
    <label>输出<select id="waveExportSource"></select></label>
    <label id="waveExportRangeLabel" hidden>时间范围<select id="waveExportRange"></select></label>
    <div class="arrange-fields">
      <label>采样率<select id="waveExportRate"><option value="48000">48 kHz</option><option value="44100">44.1 kHz</option></select></label>
      <label>位深<select id="waveExportBits"><option value="24">24 bit</option><option value="16">16 bit</option></select></label>
    </div>
    <output id="waveExportStatus" role="status"></output>
    <div class="arrange-dialog-footer">
      <button id="waveExportCancel" type="button" class="ghost" hidden>停止打包</button>
      <button id="waveExportSubmit" type="submit" class="primary">导出</button>
    </div>
  </form>`;
document.body.append(waveExportDialog);
wireDialog(waveExportDialog);
let waveExportResult=null,waveExportBeats=null,waveExportBusy=false,waveDeliveryJob=null;

function showWaveExport(result,beats=null){
  if(waveExportDialog.open||waveExportBusy)return;
  waveExportResult=result;
  waveExportBeats=beats;
  $('waveExportRangeLabel').hidden=!beats;
  const range=$('waveExportRange');
  range.replaceChildren(new Option('整首','full'));
  if(beats){
    const bounds=beats.map(beat=>String(Number(beat.toFixed(4))));
    range.add(new Option(`框选 · ${bounds[0]}–${bounds[1]} 拍`,'box'));
  }
  const options=(result.stems||[]).map((name,index)=>new Option(name.replace(/\.wav$/i,''),String(index+1)));
  $('waveExportSource').replaceChildren(new Option('立体声混音','0'),...options);
  if(options.length)$('waveExportSource').add(new Option('全部分轨 · ZIP','all'),1);
  $('waveExportStatus').textContent='';
  waveExportDialog.showModal();
}

$('waveExportClose').onclick=()=>waveExportDialog.close();
waveExportDialog.addEventListener('cancel',event=>{if(waveExportBusy)event.preventDefault()});
$('waveExportCancel').onclick=async()=>{
  if(!waveDeliveryJob)return;
  $('waveExportCancel').disabled=true;
  try{await api('render_cancel',{job_id:waveDeliveryJob})}
  catch(error){$('waveExportCancel').disabled=false;$('waveExportStatus').textContent=error.message}
};

async function waitWaveDelivery(job){
  waveDeliveryJob=job.job_id;
  const stop=$('waveExportCancel'), status=$('waveExportStatus');
  stop.hidden=false;
  for(;;){
    stop.disabled=!job.can_cancel;
    if(job.status==='complete')return job.result;
    if(job.status==='cancelled'){status.textContent='打包已取消';return null}
    if(job.status==='failed'||job.status==='interrupted')throw new Error(job.error||'分轨打包中断');
    status.textContent=job.status==='cancelling'?'正在停止打包…':
      [Math.floor((job.progress||0)*100)+'%',job.track_name].filter(Boolean).join(' · ');
    await new Promise(resolve=>setTimeout(resolve,320));
    job=await api('render_status',{job_id:waveDeliveryJob});
  }
}

$('waveExportForm').onsubmit=async event=>{
  event.preventDefault();
  const result=waveExportResult;
  if(!result||waveExportBusy)return;
  waveExportBusy=true;
  $('waveExportClose').disabled=true;
  $('waveExportSubmit').disabled=true;
  $('waveExportStatus').textContent='正在准备 WAV…';
  try{
    const output=$('waveExportSource').value;
    const all=output==='all',stemIndex=Number(output)-1;
    let saved=await api('project_export',{
      project_id:result.project_id,expected_revision:result.revision,mix_job_id:result.job_id,format:all?'wav_stems':'wav',
      sample_rate:Number($('waveExportRate').value),bit_depth:Number($('waveExportBits').value),
      ...($('waveExportRange').value==='box'&&waveExportBeats?{beat_range:waveExportBeats}:{}),
      ...(stemIndex>=0?{stem:result.stems[stemIndex]}:{})
    });
    if(all)saved=await waitWaveDelivery(saved);
    $('waveExportCancel').hidden=true;
    if(!saved)return;
    await download(saved.url);
    if(saved.warning)toast(saved.warning,true);
    waveExportDialog.close();
  }catch(error){
    $('waveExportStatus').textContent=error.message;
  }finally{
    waveExportBusy=false;
    waveDeliveryJob=null;
    $('waveExportCancel').hidden=true;
    $('waveExportClose').disabled=false;
    $('waveExportSubmit').disabled=false;
  }
};
