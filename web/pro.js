'use strict';
// Phrase selection, audio clips and portable sessions share the core revisioned API.
let selectedGroup=new Set(),groupTrack=null,marquee=null,groupDrag=null,cursorBeat=0,phraseClipboard=[],phraseClipboardLines=[],looping=false;
const coreAccept=accept;accept=function(p,external=false){if(project&&p.id===project.id&&p.revision<project.revision)return;coreAccept(p,external)};
function picked(){const ns=tr()?.notes||[];return ns.filter(n=>selectedGroup.has(n.id)||(selectedGroup.size===0&&n.id===selected))}
function chooseGroup(ids){selectedGroup=new Set(ids);groupTrack=trackId;selected=[...selectedGroup].at(-1)||null;inspect();draw()}
const coreDraw=draw;draw=function(){coreDraw();if(!project)return;if(groupTrack!==trackId){selectedGroup.clear();groupTrack=trackId}const t=tr();selectedGroup=new Set([...selectedGroup].filter(id=>t.notes.some(n=>n.id===id)));
  for(const n of picked()){ctx.strokeStyle='#d3d3d7';ctx.lineWidth=1.5;ctx.strokeRect(beatX(n.start)+.5,pitchY(n.pitch)+1.5,Math.max(3,n.duration*zoom-1),rowH-3)}ctx.lineWidth=1;
  if(marquee){const {x,y,xx,yy}=marquee;ctx.fillStyle='#d0d0d018';ctx.fillRect(Math.min(x,xx),Math.min(y,yy),Math.abs(xx-x),Math.abs(yy-y));ctx.strokeStyle='#bdbdc1';ctx.strokeRect(Math.min(x,xx),Math.min(y,yy),Math.abs(xx-x),Math.abs(yy-y))}
  if(audio.paused&&cursorBeat>0){ctx.strokeStyle='#cbcbcf';ctx.beginPath();ctx.moveTo(beatX(cursorBeat),0);ctx.lineTo(beatX(cursorBeat),roll.clientHeight);ctx.stroke()}
  if(isAudioTrack(t)){
    $('emptyHint').hidden=true;
    const view=$('rollViewport');
    const y=view.scrollTop+Math.max(rulerH+14,(view.clientHeight-110)/2);
    ctx.save();ctx.beginPath();ctx.rect(keyW,rulerH,roll.clientWidth-keyW,roll.clientHeight-rulerH);ctx.clip();
    for(const clip of trackClips(t))paintClipWave(ctx,t,clip,beatX,y,110,view.scrollLeft+keyW,view.scrollLeft+view.clientWidth,draw);
    ctx.restore();
  }
  const count=picked().length;if(count>1)$('selectedIndicator').textContent=count+' 音符';
};
const coreBeats=beats;beats=function(){
  const t=tr();
  return isAudioTrack(t)?Math.max(coreBeats(),Math.ceil((Math.max(0,...trackClips(t).map(clipEndBeat))+4)/4)*4):coreBeats();
};
const audioSettings=document.createElement('div');
audioSettings.className='audio-settings';
audioSettings.innerHTML='<button id="audioStart" class="subtle" type="button">在编排中编辑片段</button>';
audioSettings.hidden=true;$('trackName').parentElement.after(audioSettings);
$('audioStart').onclick=()=>void switchArrangement(true);
const coreRefresh=refresh;refresh=function(external=false){
  coreRefresh(external);
  if(!tr())return;
  audioSettings.hidden=!isAudioTrack(tr());
  if(isAudioTrack(tr())){$('emptyHint').hidden=true;$('batchLyrics').disabled=true}
  else $('batchLyrics').disabled=false;
};
roll.addEventListener('pointerdown',e=>{if(!project||e.button!==0)return;const pt=point(e),viewport=$('rollViewport').getBoundingClientRect();if(e.clientY<viewport.top+rulerH&&e.clientX>=viewport.left+keyW){e.stopImmediatePropagation();cursorBeat=Math.max(0,snap((pt.x-keyW)/zoom));if(window.desktopNative||audio.duration&&Number.isFinite(audio.duration))audio.currentTime=Math.min(audio.duration,beatSeconds(cursorBeat));draw();return}
  if(isAudioTrack(tr())){e.stopImmediatePropagation();return}const n=hit(pt.x,pt.y);
  if(n&&(e.shiftKey||e.ctrlKey||e.metaKey)){e.stopImmediatePropagation();if(groupTrack!==trackId)selectedGroup.clear();groupTrack=trackId;if(selectedGroup.has(n.id))selectedGroup.delete(n.id);else selectedGroup.add(n.id);selected=n.id;inspect();draw();return}
  if(n&&selectedGroup.has(n.id)&&selectedGroup.size>1&&pt.x<beatX(n.start+n.duration)-10){e.stopImmediatePropagation();groupDrag={x:pt.x,y:pt.y,originals:picked().map(n=>structuredClone(n)),moved:false};drag={pro:true};roll.setPointerCapture(e.pointerId);return}
  if(n){selectedGroup=new Set([n.id]);groupTrack=trackId;return}
  if(pt.x>=keyW&&pt.y>=rulerH){e.stopImmediatePropagation();marquee={x:pt.x,y:pt.y,xx:pt.x,yy:pt.y,append:e.shiftKey};drag={pro:true};if(!e.shiftKey)selectedGroup.clear();selected=null;roll.setPointerCapture(e.pointerId);inspect();draw()}
},true);
roll.addEventListener('pointermove',e=>{if(marquee){e.stopImmediatePropagation();const pt=point(e);marquee.xx=pt.x;marquee.yy=pt.y;draw()}else if(groupDrag){e.stopImmediatePropagation();const pt=point(e),d=groupDrag;const offset=snap((pt.x-d.x)/zoom),pitch=-Math.round((pt.y-d.y)/rowH);d.moved||=Math.abs(pt.x-d.x)+Math.abs(pt.y-d.y)>3;for(const orig of d.originals){const n=tr().notes.find(n=>n.id===orig.id);n.start=Math.max(0,orig.start+offset);n.pitch=Math.max(0,Math.min(127,orig.pitch+pitch))}draw()}},true);
roll.addEventListener('pointerup',e=>{if(marquee||groupDrag)drag=null;if(marquee){e.stopImmediatePropagation();const r=marquee;marquee=null;if(Math.abs(r.xx-r.x)>3||Math.abs(r.yy-r.y)>3){const a=Math.min(r.x,r.xx),b=Math.max(r.x,r.xx),c=Math.min(r.y,r.yy),d=Math.max(r.y,r.yy);const ns=tr().notes.filter(n=>beatX(n.start+n.duration)>a&&beatX(n.start)<b&&pitchY(n.pitch)+rowH>c&&pitchY(n.pitch)<d);chooseGroup([...selectedGroup,...ns.map(n=>n.id)])}else{cursorBeat=Math.max(0,snap((r.x-keyW)/zoom));inspect();draw()}}
  else if(groupDrag){e.stopImmediatePropagation();const d=groupDrag;groupDrag=null;const ops=d.originals.map(orig=>{const n=tr().notes.find(n=>n.id===orig.id);const values={start:n.start,pitch:n.pitch};Object.assign(n,orig);return{op:'update_note',track_id:trackId,note_id:n.id,values}});if(d.moved)edit(ops).catch(()=>{});else draw()}
},true);
roll.addEventListener('pointercancel',()=>{if(marquee||groupDrag)drag=null;marquee=null;if(groupDrag){for(const o of groupDrag.originals)Object.assign(tr().notes.find(n=>n.id===o.id),o);groupDrag=null}draw()},true);
roll.addEventListener('dblclick',e=>{const viewport=$('rollViewport').getBoundingClientRect();if(e.clientY<viewport.top+rulerH||e.clientX<viewport.left+keyW){e.stopImmediatePropagation();return}if(isAudioTrack(tr())){e.stopImmediatePropagation();toast('这是一条伴奏音轨；请选择主唱或和声音轨写入音符。')}},true);
function transformNotes(fn){const ns=picked();if(!ns.length){toast('先选择音符；Shift 点击可多选，拖动空白处可框选');return}edit(ns.map(n=>({op:'update_note',track_id:trackId,note_id:n.id,values:fn(n)}))).catch(()=>{})}
function transpose(delta){transformNotes(n=>({pitch:n.pitch+delta}))}
$('transposeDown').onclick=()=>transpose(-1);$('transposeUp').onclick=()=>transpose(1);$('quantizeNotes').onclick=()=>transformNotes(n=>({start:snap(n.start)}));
function copyPhrase(){
  const ns=picked(),track=tr();
  if(!ns.length)return false;
  const first=Math.min(...ns.map(note=>note.start));
  phraseClipboard=ns.map(({id,...note})=>({...structuredClone(note),start:note.start-first}));
  phraseClipboardLines=[];
  if(track.lyric_lines?.length){
    const clipboardPositions=new Map(ns.map((note,i)=>[note.id,i]));
    const trackPositions=new Map(track.notes.map((note,i)=>[note.id,i]));
    for(const line of track.lyric_lines){
      const indexes=line.note_ids.map(id=>clipboardPositions.get(id));
      if(indexes.some(index=>index===undefined))continue;
      const start=trackPositions.get(line.note_ids[0]);
      if(track.notes[start].vowel_continuation||track.notes[start+indexes.length]?.vowel_continuation)continue;
      if(line.note_ids.some((id,i)=>trackPositions.get(id)!==start+i||
        line.reading[i]!== (ns[indexes[i]].vowel_continuation?'+':ns[indexes[i]].lyric)))continue;
      phraseClipboardLines.push({text:line.text,indexes});
    }
  }
  return true;
}
function pastePhrase(at){
  if(!phraseClipboard.length||isAudioTrack(tr()))return;
  const target=trackId,existing=new Set(tr().notes.map(note=>note.id));
  const notes=phraseClipboard.map(note=>({...structuredClone(note),start:note.start+at}));
  const ops=[{op:'add_notes',track_id:target,notes}];
  if(!tr().instrument&&phraseClipboardLines.length){
    for(const note of notes){
      const bytes=crypto.getRandomValues(new Uint8Array(16));
      note.id='n_'+Array.from(bytes,byte=>byte.toString(16).padStart(2,'0')).join('');
    }
    for(const line of phraseClipboardLines)ops.push({op:'set_lyric_line',track_id:target,
      note_ids:line.indexes.map(index=>notes[index].id),text:line.text});
  }
  edit(ops).then(()=>{if(trackId===target)chooseGroup(tr().notes.filter(note=>!existing.has(note.id)).map(note=>note.id))}).catch(()=>{});
}
$('duplicateNotes').onclick=()=>{const ns=picked();if(!ns.length||!copyPhrase())return;const at=Math.ceil(Math.max(...ns.map(n=>n.start+n.duration))/Number($('quantize').value))*Number($('quantize').value);pastePhrase(at)};
$('deleteNote').onclick=()=>{const ns=picked();if(ns.length)edit([{op:'delete_notes',track_id:trackId,note_ids:ns.map(n=>n.id)}]).catch(()=>{})};
function loopRange(){
  const clip=looping&&typeof arrangementVisible!=='undefined'&&arrangementVisible&&typeof selectedAudioClip==='function'
    ? selectedAudioClip()?.clip : null;
  if(clip){
    const frame=clipStartFrame(clip);
    return {start:frame/clipRate,end:clipEndFrame(clip)/clipRate};
  }
  if(looping&&typeof arrangementVisible!=='undefined'&&arrangementVisible&&arrangeSelection)
    return {start:beatSeconds(arrangeSelection.start),end:beatSeconds(arrangeSelection.end)};
  const notes=looping?picked():[];
  if(!notes.length)return null;
  return {start:beatSeconds(Math.min(...notes.map(note=>note.start))),end:beatSeconds(Math.max(...notes.map(note=>note.start+note.duration)))};
}
function loopTime(time){
  const range=loopRange();
  return range&&(time<range.start||time>=range.end)?range.start:time;
}
$('loop').onclick=()=>{
  looping=!looping;
  $('loop').classList.toggle('on',looping);
  $('loop').setAttribute('aria-pressed',String(looping));
  if(looping&&!loopRange())toast('选择音符或片段设定循环范围');
  if(window.desktopNative)audio.setUserLoop(loopRange());
};
audio.addEventListener('timeupdate',()=>{
  if(window.desktopNative||audio.paused||audio.seeking)return;
  const target=loopTime(audio.currentTime);
  if(target!==audio.currentTime)audio.currentTime=target;
});

importFile=async function(file){
  if(!file)return;
  try{
    if(file.size>128e6)throw new Error('文件超过 128 MB');
    await editChain.catch(()=>{});
    const data=new Uint8Array(await file.arrayBuffer());
    let binary='';
    for(let i=0;i<data.length;i+=32768)binary+=String.fromCharCode(...data.subarray(i,i+32768));
    const args={filename:file.name,data_base64:btoa(binary)};
    if(audioImportExtension.test(file.name)){
      if(!project)throw new Error('先新建一个工程');
      const projectId=project.id;
      const saved=await api('audio_import',{...args,project_id:projectId,expected_revision:project.revision,
        start:cursorBeat,name:('伴奏 · '+file.name.replace(audioImportExtension,'')).slice(0,100)});
      if(project?.id===projectId){
        trackId=saved.tracks.at(-1).id;
        selectedGroup.clear();
        accept(saved);
        toast('伴奏已导入为独立音轨');
      }else toast('音频已添加到原工程');
    }else{
      const imported=await importScore(args,file.name);
      if(!imported)return;
      selectedGroup.clear();
      accept(imported.project);
      toast(imported.warnings?.length?imported.warnings.join(' '):'曲谱已导入为新工程');
    }
    await refreshList();
  }catch(error){toast(error.message,true)}
};
$('exportProject').onclick=()=>exportFile('avpack');
audio.addEventListener('timeupdate',()=>{const s=audio.currentTime;$('timeDisplay').textContent=String(Math.floor(s/60)).padStart(2,'0')+':'+(s%60).toFixed(2).padStart(5,'0')});
setInterval(()=>refreshList().catch(()=>{}),7000);
let selectionPublishTimer;
function currentStudioSelection(){
  const inArrangement=typeof workspaceFocus!=='undefined'&&workspaceFocus==='arrangement';
  const box=inArrangement&&arrangeSelection;
  return {note_ids:inArrangement?arrangeSelectionIds():picked().map(n=>n.id),
    clip_refs:inArrangement?clipSelectionRefs():[],
    ...(box?{arrangement_range:{track_ids:[...box.ids],beat_range:[box.start,box.end]}}:{})};
}
const pitchJoinButton=document.createElement('button');
pitchJoinButton.id='pitchJoin';
pitchJoinButton.type='button';
pitchJoinButton.textContent='滑音衔接';
$('velocityTab').after(pitchJoinButton);
const pitchReferenceButton=document.createElement('button');
pitchReferenceButton.id='pitchReference';
pitchReferenceButton.type='button';
pitchReferenceButton.textContent='参考调声';
pitchJoinButton.after(pitchReferenceButton);
const pitchVibratoButton=document.createElement('button');
pitchVibratoButton.id='pitchVibrato';
pitchVibratoButton.type='button';
pitchVibratoButton.textContent='颤音';
pitchJoinButton.after(pitchVibratoButton);
const vibratoShapeFields={
  vibratoShapeDepth:['depth',1,0],vibratoShapeRate:['rate',1,5.5],
  vibratoShapeStart:['start',100,.6],vibratoShapeEnd:['end',100,1],
  vibratoShapeFadeIn:['fade_in_ms',1,80],vibratoShapeFadeOut:['fade_out_ms',1,0]
};
let pitchJoinTarget=null,pitchJoinPlan=null,pitchJoinBusy=false;
let pitchReferenceChanges=[];
let pitchAudio=null;

function paintPitchAudio(){
  const ready=pitchAudio?.status==='complete';
  const playing=ready&&audio.auditionId===pitchAudio.id&&!audio.paused;
  $('pitchAudition').hidden=!window.desktopNative||!pitchJoinPlan;
  $('pitchAuditionStart').disabled=pitchJoinBusy||!!pitchAudio&&!ready;
  $('pitchAuditionA').disabled=$('pitchAuditionB').disabled=!ready||pitchJoinBusy;
  $('pitchAuditionA').setAttribute('aria-pressed',String(playing&&audio.auditionVariant===0));
  $('pitchAuditionB').setAttribute('aria-pressed',String(playing&&audio.auditionVariant===1));
  $('pitchAuditionStop').disabled=!pitchAudio;
  $('pitchLocalPreview').disabled=pitchJoinBusy;
}

async function dropPitchAudio(job=pitchAudio){
  if(!job)return;
  if(pitchAudio===job){
    pitchAudio=null;
    paintPitchAudio();
  }
  clearTimeout(job.timer);
  if(!job.id)return;
  await audio.stopAudition(job.id).catch(()=>{});
  await api('pitch_audition_release',{audition_id:job.id}).catch(()=>{});
}

function pitchAudioCurrent(job){
  return pitchAudio===job&&job.plan===pitchJoinPlan&&$('pitchJoinDialog').open&&
    project?.id===job.target.project_id&&project.revision===job.target.expected_revision;
}

async function playPitchAudio(side){
  const job=pitchAudio;
  if(job?.status!=='complete')return;
  if(!pitchAudioCurrent(job)){
    await dropPitchAudio(job);
    if($('pitchJoinDialog').open)pitchJoinState('工程已更新，请重新预览');
    return;
  }
  try{await audio.auditionSide(job.id,side)}
  catch(error){if(pitchAudioCurrent(job))pitchJoinState(error.message)}
}

async function pollPitchAudio(job){
  try{
    if(!pitchAudioCurrent(job)){await dropPitchAudio(job);return}
    const state=await api('pitch_audition_status',{audition_id:job.id});
    if(!pitchAudioCurrent(job)){await dropPitchAudio(job);return}
    job.status=state.status;
    if(state.status==='complete'){
      pitchJoinState(job.target.reference?'A 原曲线合成 · B 参考曲线合成':
        job.target.vibrato?'A 原版 · B 颤音':'A 原版 · B 滑音');
      await playPitchAudio('B');
    }else if(['running','cancelling','committing'].includes(state.status)){
      job.timer=setTimeout(()=>void pollPitchAudio(job),230);
    }else{
      await dropPitchAudio(job);
      pitchJoinState(state.error||'试听未生成');
    }
  }catch(error){
    if(pitchAudio===job)await dropPitchAudio(job);
    if($('pitchJoinDialog').open&&pitchJoinPlan===job.plan)pitchJoinState(error.message);
  }
}

$('pitchAuditionStart').onclick=async()=>{
  if(!pitchJoinPlan||pitchJoinBusy||pitchAudio&&pitchAudio.status!=='complete')return;
  if(pitchAudio?.status==='complete'){await playPitchAudio('B');return}
  const job={target:pitchJoinTarget,plan:pitchJoinPlan,status:'starting'};
  pitchAudio=job;
  pitchJoinState('正在准备 A/B 试听…');
  try{
    const state=await api('pitch_audition_start',{...job.plan,engine:$('renderEngine').value,local_voice:$('pitchLocalPreview').checked});
    job.id=state.job_id;
    await pollPitchAudio(job);
  }catch(error){
    if(pitchAudio===job){
      await dropPitchAudio(job);
      pitchJoinState(error.message);
    }
  }
};
$('pitchAuditionA').onclick=()=>void playPitchAudio('A');
$('pitchAuditionB').onclick=()=>void playPitchAudio('B');
$('pitchAuditionStop').onclick=()=>{void dropPitchAudio();pitchJoinState()};
$('pitchLocalPreview').onchange=()=>{void dropPitchAudio();pitchJoinState()};
for(const event of ['timeupdate','pause','ended'])audio.addEventListener(event,()=>{
  if(pitchAudio||$('pitchJoinDialog').open)paintPitchAudio();
});

function pitchJoinState(message){
  $('pitchJoinStatus').textContent=message||pitchJoinTarget?.label||'';
  const reference=!!pitchJoinTarget?.reference;
  const vibrato=!!pitchJoinTarget?.vibrato;
  const draft=reference?'参考曲线':vibrato?'颤音':'滑音';
  $('previewPitchJoin').disabled=pitchJoinBusy||reference&&!$('pitchReferenceSource').value;
  $('applyPitchJoin').disabled=pitchJoinBusy||!pitchJoinPlan;
  $('pitchJoinLead').readOnly=$('pitchJoinTrail').readOnly=pitchJoinBusy;
  $('pitchEdgeLength').readOnly=$('pitchEdgeOffset').readOnly=pitchJoinBusy;
  $('pitchEdgeSide').disabled=pitchJoinBusy;
  $('pitchReferenceSource').disabled=pitchJoinBusy;
  for(const id of Object.keys(vibratoShapeFields))$(id).readOnly=pitchJoinBusy;
  $('pitchReferenceNoteField').hidden=!reference||pitchReferenceChanges.length<2;
  $('pitchDraftLegend').textContent=draft;
  $('pitchAuditionA').textContent=reference?'A 原曲线':'A 原版';
  $('pitchAuditionB').textContent='B '+draft;
  $('applyPitchJoin').textContent='应用'+(reference?'曲线':draft);
  $('pitchJoinPreview').hidden=!pitchJoinPlan;
  paintPitchAudio();
}

pitchJoinButton.onclick=async()=>{
  try{
    await editChain;
    const ids=currentStudioSelection().note_ids;
    const track=project?.tracks.find(candidate=>ids.every(id=>candidate.notes.some(note=>note.id===id)));
    if(![1,2].includes(ids.length)||!track||track.instrument||isAudioTrack(track))
      throw new Error('请选择一个歌声音符，或同轨中两个相接的音符');
    const [left,right]=track.notes.filter(note=>ids.includes(note.id)).sort((a,b)=>a.start-b.start);
    const boundary=left.start+left.duration;
    const clock=clockFor(project);
    const start=clock.seconds(left.start),join=clock.seconds(boundary);
    const base={project_id:project.id,expected_revision:project.revision,note_ids:ids,clock};
    if(right){
      if(Math.abs(boundary-right.start)>1e-7)throw new Error('两个歌声音符必须首尾相接');
      const available=[join-start,clock.seconds(right.start+right.duration)-join];
      $('pitchJoinLead').value=Math.min(45,Math.floor(available[0]*1000));
      $('pitchJoinTrail').value=Math.min(70,Math.floor(available[1]*1000));
      pitchJoinTarget={...base,origin:join,available,label:track.name+' · '+noteName(left.pitch)+' → '+noteName(right.pitch)};
    }else{
      const duration=join-start,limit=Math.min(2000,Math.floor(duration*1000+1e-6));
      if(limit<20)throw new Error('音符不足 20 毫秒');
      $('pitchEdgeSide').value='start';
      $('pitchEdgeLength').max=limit;
      $('pitchEdgeLength').value=Math.min(85,limit);
      $('pitchEdgeOffset').value=-75;
      pitchJoinTarget={...base,edge:true,origin:start,duration,label:track.name+' · '+noteName(left.pitch)};
    }
    $('pitchJoinTitle').textContent=right?'滑音衔接':'起尾滑音';
    $('pitchJoinFields').hidden=!right;
    $('pitchEdgeFields').hidden=!!right;
    $('pitchReferenceFields').hidden=$('pitchVibratoFields').hidden=true;
    $('pitchLocalPreview').checked=false;
    pitchJoinPlan=null;
    pitchJoinState();
    $('pitchJoinDialog').showModal();
    $('previewPitchJoin').click();
  }catch(error){toast(error.message,true)}
};

pitchVibratoButton.onclick=async()=>{
  try{
    await editChain;
    const ids=currentStudioSelection().note_ids;
    const track=project?.tracks.find(candidate=>candidate.notes.some(note=>note.id===ids[0]));
    if(ids.length!==1||!track||track.instrument||isAudioTrack(track))throw new Error('请选择一个歌声音符');
    const note=track.notes.find(note=>note.id===ids[0]);
    for(const [id,[key,scale,fallback]] of Object.entries(vibratoShapeFields))
      $(id).value=(note.vibrato[key]??fallback)*scale;
    pitchJoinTarget={project_id:project.id,expected_revision:project.revision,note_ids:ids,
      clock:clockFor(project),vibrato:true,label:track.name+' · '+note.lyric+' · '+noteName(note.pitch)};
    $('pitchJoinTitle').textContent='颤音';
    $('pitchJoinFields').hidden=$('pitchEdgeFields').hidden=$('pitchReferenceFields').hidden=true;
    $('pitchVibratoFields').hidden=false;
    $('pitchLocalPreview').checked=true;
    pitchJoinPlan=null;
    pitchJoinState();
    $('pitchJoinDialog').showModal();
  }catch(error){toast(error.message,true)}
};

pitchReferenceButton.onclick=async()=>{
  try{
    await editChain;
    const ids=currentStudioSelection().note_ids;
    const track=project?.tracks.find(candidate=>ids.every(id=>candidate.notes.some(note=>note.id===id)));
    if(!ids.length||ids.length>32||!track||track.instrument||isAudioTrack(track))
      throw new Error('请选择同一歌声轨上的 1–32 个音符');
    const notes=track.notes.filter(note=>ids.includes(note.id));
    const start=Math.min(...notes.map(note=>note.start));
    const end=Math.max(...notes.map(note=>note.start+note.duration));
    const clock=clockFor(project);
    if(clock.seconds(end)-clock.seconds(start)>15)throw new Error('参考调声选区最多 15 秒');
    const options=project.tracks.flatMap(source=>trackClips(source)
      .filter(clip=>clip.beat_length==null&&clip.start<=start+1e-8&&clipEndBeat(clip)>=end-1e-8)
      .map(clip=>({track_id:source.id,clip_id:clip.id,label:clip.id==='legacy'?source.name:source.name+' · '+clip.name,
        preferred:source.name==='原版人声'})));
    if(!options.length)throw new Error('需要一条覆盖选区的固定秒数干声片段');
    const select=$('pitchReferenceSource');
    select.replaceChildren(new Option('选择参考片段',''),...options.map((clip,index)=>new Option(clip.label,String(index))));
    const suggested=options.findIndex(clip=>clip.preferred);
    if(suggested>=0)select.value=String(suggested);
    pitchJoinTarget={project_id:project.id,expected_revision:project.revision,note_ids:ids,clock,reference:options,
      label:track.name+' · '+ids.length+' 个音符'};
    $('pitchJoinTitle').textContent='参考调声';
    $('pitchJoinFields').hidden=$('pitchEdgeFields').hidden=$('pitchVibratoFields').hidden=true;
    $('pitchReferenceFields').hidden=false;
    $('pitchLocalPreview').checked=true;
    pitchJoinPlan=null;pitchReferenceChanges=[];
    pitchJoinState();
    $('pitchJoinDialog').showModal();
  }catch(error){toast(error.message,true)}
};
$('pitchJoinDialog').addEventListener('close',()=>{pitchJoinTarget=null;pitchJoinPlan=null;pitchReferenceChanges=[];void dropPitchAudio()});
function resetPitchReference(){
  pitchReferenceChanges=[];
  pitchJoinPlan=null;
  void dropPitchAudio();
  pitchJoinState();
}
$('pitchReferenceSource').onchange=resetPitchReference;
$('pitchReferenceNote').onchange=()=>drawPitchReference();
for(const id of ['pitchJoinLead','pitchJoinTrail','pitchEdgeSide','pitchEdgeLength','pitchEdgeOffset',...Object.keys(vibratoShapeFields)])
  $(id).oninput=()=>{pitchJoinPlan=null;void dropPitchAudio();pitchJoinState()};

function drawPitchJoin(target,transform,changes){
  $('pitchVibratoWindow').setAttribute('visibility','hidden');
  let first,last,sides,marker=0;
  if(target.edge){
    const width=transform.length_ms/1000+.045;
    marker=transform.side==='start'?0:target.duration;
    first=marker===0?0:Math.max(0,target.duration-width);
    last=marker===0?Math.min(target.duration,width):target.duration;
    sides=[[changes[0],first,last]];
  }else{
    const [left,right]=changes.slice().sort((a,b)=>a.before.start-b.before.start);
    first=-Math.min(target.available[0],transform.lead_ms/1000+.045);
    last=Math.min(target.available[1],transform.trail_ms/1000+.045);
    sides=[[left,first,0],[right,0,last]];
  }
  const values=field=>sides.flatMap(([change,start,end])=>Array.from({length:41},(_,index)=>{
    const time=start+(end-start)*index/40,note=change[field];
    const beat=Math.max(0,Math.min(note.duration,target.clock.beat(target.origin+time)-note.start));
    return [time,note.pitch*100+curveCents(note,beat)];
  }));
  const before=values('before'),after=values('after');
  const pitches=[...before,...after].map(point=>point[1]);
  const center=(Math.min(...pitches)+Math.max(...pitches))/2,range=Math.max(100,Math.max(...pitches)-Math.min(...pitches));
  const x=time=>10+412*(time-first)/(last-first);
  const points=series=>series.map(([time,pitch])=>x(time).toFixed(2)+','+(56-86*(pitch-center)/range).toFixed(2)).join(' ');
  $('pitchJoinMarker').removeAttribute('visibility');
  $('pitchJoinMarker').setAttribute('x1',x(marker));
  $('pitchJoinMarker').setAttribute('x2',x(marker));
  $('pitchJoinOriginal').setAttribute('points',points(before));
  $('pitchJoinDraft').setAttribute('points',points(after));
}

function drawPitchReference(){
  const change=pitchReferenceChanges[Number($('pitchReferenceNote').value)||0];
  if(!change)return;
  const duration=change.before.duration;
  const curves=['before','after'].map(field=>Array.from({length:101},(_,i)=>curveCents(change[field],duration*i/100)));
  const low=Math.min(...curves.flat()),high=Math.max(...curves.flat());
  const mid=(low+high)/2,scale=86/Math.max(100,high-low);
  $('pitchJoinMarker').setAttribute('visibility','hidden');
  $('pitchVibratoWindow').setAttribute('visibility','hidden');
  for(const [index,id] of ['pitchJoinOriginal','pitchJoinDraft'].entries()){
    $(id).setAttribute('points',curves[index].map((cents,i)=>[10+i*4.12,56-(cents-mid)*scale].map(n=>n.toFixed(2)).join(',')).join(' '));
  }
}

function drawVibratoEnvelope(clock,change,envelope){
  const {before,after}=change;
  const start=clock.seconds(before.start);
  const length=clock.seconds(before.start+before.duration)-start;
  const seconds=beat=>clock.seconds(beat);
  const samples=Math.max(120,Math.min(960,Math.ceil(length*100)));
  const curves=[before,after].map(note=>Array.from({length:samples+1},(_,index)=>{
    const beat=Math.max(0,Math.min(note.duration,clock.beat(start+length*index/samples)-note.start));
    return noteCents(note,beat,seconds);
  }));
  const low=Math.min(...curves.flat()),high=Math.max(...curves.flat());
  const center=(low+high)/2,scale=86/Math.max(100,high-low);
  for(const [index,id] of ['pitchJoinOriginal','pitchJoinDraft'].entries()){
    $(id).setAttribute('points',curves[index].map((cents,i)=>
      (10+412*i/samples).toFixed(2)+','+(56-(cents-center)*scale).toFixed(2)).join(' '));
  }
  $('pitchJoinMarker').setAttribute('visibility','hidden');
  const [from,to]=envelope.note_offset_ms.map(ms=>412*ms/(length*1000));
  const shade=$('pitchVibratoWindow');
  shade.setAttribute('visibility',envelope.cycle_count>0?'visible':'hidden');
  shade.setAttribute('x',10+from);
  shade.setAttribute('width',to-from);
}

$('previewPitchJoin').onclick=async()=>{
  const target=pitchJoinTarget;
  if(!target||pitchJoinBusy)return;
  void dropPitchAudio();
  pitchJoinBusy=true;
  pitchJoinPlan=null;
  pitchJoinState(target.reference?'正在分析干声音高…':target.vibrato?'正在计算颤音…':'正在计算滑音…');
  let message;
  try{
    if(project?.id!==target.project_id||project.revision!==target.expected_revision)throw new Error('工程已更新，请重新选择音符');
    let transform=target.edge?{kind:'pitch_edge',side:$('pitchEdgeSide').value,
      length_ms:$('pitchEdgeLength').valueAsNumber,offset_cents:$('pitchEdgeOffset').valueAsNumber}:
      {kind:'pitch_join',lead_ms:$('pitchJoinLead').valueAsNumber,trail_ms:$('pitchJoinTrail').valueAsNumber};
    if(target.reference){
      const index=$('pitchReferenceSource').value;
      if(index==='')throw new Error('请选择对齐的干声片段');
      const {track_id,clip_id}=target.reference[Number(index)];
      transform={kind:'pitch_reference',reference:{track_id,clip_id}};
    }else if(target.vibrato){
      const settings={};
      for(const [id,[key,scale]] of Object.entries(vibratoShapeFields)){
        const input=$(id);
        if(!input.validity.valid||!Number.isFinite(input.valueAsNumber))throw new Error('请输入有效的颤音参数');
        settings[key]=input.valueAsNumber/scale;
      }
      transform={kind:'vibrato_envelope',settings};
    }
    if(Object.values(transform).some(value=>typeof value==='number'&&!Number.isFinite(value)))throw new Error('请输入滑音参数');
    const selection=await api('selection_resolve',{project_id:target.project_id,note_ids:target.note_ids});
    const plan={selection_id:selection.selection_id,expected_revision:target.expected_revision,transform};
    const preview=await api('selection_edit',plan);
    if(pitchJoinTarget!==target)return;
    if(target.reference){
      pitchReferenceChanges=preview.changes;
      if(!preview.change_count)throw new Error('选区没有可靠的有声音高');
      $('pitchReferenceNote').replaceChildren(...preview.changes.map(({before},i)=>
        new Option(noteName(before.pitch)+' · '+before.lyric+' · '+before.start.toFixed(2)+' 拍',String(i))));
      drawPitchReference();
      const skipped=target.note_ids.length-preview.change_count;
      message=`${preview.change_count} 个音符可应用`+(skipped?` · ${skipped} 个略过`:'');
    }else if(target.vibrato){
      const envelope=preview.vibrato_envelope;
      drawVibratoEnvelope(target.clock,preview.changes[0],envelope);
      const [from,to]=envelope.note_offset_ms.map(Math.round);
      message=envelope.cycle_count>0?`${from}–${to} ms · ${envelope.cycle_count.toFixed(1)} 周期`:'颤音关闭';
    }else drawPitchJoin(target,transform,preview.changes);
    pitchJoinPlan=plan;
  }catch(error){message=error.message}
  finally{pitchJoinBusy=false;pitchJoinState(pitchJoinTarget===target?message:null)}
};
$('applyPitchJoin').onclick=async()=>{
  const target=pitchJoinTarget,plan=pitchJoinPlan;
  if(!target||!plan||pitchJoinBusy)return;
  const heard=pitchAudio?.status==='complete'&&pitchAudioCurrent(pitchAudio)?pitchAudio.id:null;
  pitchJoinBusy=true;
  pitchJoinState(target.reference?'正在保存参考曲线…':target.vibrato?'正在保存颤音…':'正在保存滑音…');
  let message;
  try{
    if(!heard)await dropPitchAudio();
    await editChain;
    const request=heard?{selection_id:plan.selection_id,expected_revision:plan.expected_revision,audition_id:heard}:plan;
    const saved=await api('selection_edit',{...request,dry_run:false});
    if(heard)await dropPitchAudio();
    if(project?.id===saved.project.id)accept(saved.project);
    if(pitchJoinTarget===target)$('pitchJoinDialog').close();
    await refreshList();
  }catch(error){pitchJoinPlan=null;message=error.message}
  finally{pitchJoinBusy=false;pitchJoinState(pitchJoinTarget===target?message:null)}
};
function publishSelection(){
  clearTimeout(selectionPublishTimer);
  selectionPublishTimer=setTimeout(()=>{
    if(!project||drag||curveDrag)return;
    api('ui_selection_set',{project_id:project.id,expected_revision:project.revision,...currentStudioSelection()}).catch(()=>{});
  },180);
}
const beforeSelectionGroup=chooseGroup;chooseGroup=function(ids){beforeSelectionGroup(ids);publishSelection()};const beforeSelectionRefresh=refresh;refresh=function(external=false){beforeSelectionRefresh(external);publishSelection()};roll.addEventListener('pointerup',()=>setTimeout(publishSelection,0));document.addEventListener('keyup',publishSelection);
const singleNoteUpdate=updateNote;updateNote=function(values){const ns=picked(),anchor=note();if(ns.length<=1||!anchor)return singleNoteUpdate(values);const ops=ns.map(n=>{const next=structuredClone(values);if('pitch'in next)next.pitch=n.pitch+values.pitch-anchor.pitch;if('start'in next)next.start=n.start+values.start-anchor.start;return{op:'update_note',track_id:trackId,note_id:n.id,values:next}});return edit(ops)};
