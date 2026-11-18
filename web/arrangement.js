'use strict';

const scorePanel=document.querySelector('.editor-shell');
const arrangementPanel=document.createElement('section');
arrangementPanel.id='arrangementPanel';
arrangementPanel.innerHTML=`
  <div class="arrange-toolbar">
    <strong>编排</strong><output id="arrangeSpan" aria-live="polite"></output>
    <div class="arrange-actions">
      <button id="arrangeRange" title="设置拍范围与音轨">${icon('grid')} 范围</button>
      <button id="arrangeCopy" title="复制选定音符">${icon('copy')} 复制</button>
      <button id="arrangeMove" title="移动选定音符">${icon('right')} 移动</button>
      <button id="arrangeTranspose" title="同时移调所选歌声和乐器">${icon('pitch')} 移调</button>
      <button id="arrangeQuantize" title="量化所选轨道的音符起点 (Q)">量化</button>
      <button id="arrangeClear" title="清除选定音符">${icon('close')} 清除</button>
    </div>
    <div class="arrange-zoom">
      <button id="editSections" type="button" aria-label="编辑段落标记" title="段落标记">${icon('flag')}</button>
      <button id="arrangeZoomOut" aria-label="缩小编排">${icon('minus')}</button>
      <button id="arrangeZoomIn" aria-label="放大编排">${icon('plus')}</button>
    </div>
  </div>
  <div id="arrangeViewport" tabindex="0" aria-label="多轨编排，拖动框选音轨和拍范围，双击进入音轨">
    <div id="arrangeExtent" aria-hidden="true"></div>
    <canvas id="arrangeCanvas" aria-label="钢琴卷帘的多轨编排总览"></canvas>
  </div>`;
scorePanel.before(arrangementPanel);
const viewButton=document.createElement('button');
viewButton.id='arrangeToggle';
viewButton.className='arrange-toggle';
viewButton.setAttribute('aria-pressed','true');
viewButton.title='显示或收起下方编辑器 (F2)';
viewButton.innerHTML=icon('grid')+'<span>编辑器</span>';
arrangementPanel.querySelector('.arrange-zoom').after(viewButton);
viewButton.onclick=()=>setDetailsVisible(scorePanel.hidden);

const rangeDialog=document.createElement('dialog');
rangeDialog.id='arrangeDialog';
rangeDialog.className='arrange-dialog';
rangeDialog.innerHTML=`
  <div class="dialog-heading"><h2 id="arrangeTitle">范围</h2><button id="arrangeClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="arrange-fields">
    <label>起点 · 拍<input id="arrangeFrom" type="number" min="0" step="any"></label>
    <label>终点 · 拍<input id="arrangeTo" type="number" min="0" step="any"></label>
    <label id="arrangeTargetLabel">目标起点 · 拍<input id="arrangeTarget" type="number" min="0" step="any"></label>
    <label id="arrangeShiftLabel" hidden>移调 · 半音<input id="arrangeShift" type="number" min="-127" max="127" step="1"></label>
  </div>
  <div id="arrangeTimingFields" class="arrange-fields" hidden>
    <label>网格<select id="arrangeGrid"><option value="1">1/4</option><option value="0.5">1/8</option><option value="0.3333333333333333">1/8 三连音</option><option value="0.25">1/16</option><option value="0.16666666666666666">1/16 三连音</option><option value="0.125">1/32</option></select></label>
    <label>强度 · %<input id="arrangeStrength" type="number" min="1" max="100" step="1" value="75"></label>
    <label>摇摆 · %<input id="arrangeSwing" type="number" min="50" max="75" step="1" value="50" title="50% 为等分网格。数值越大，成对网格的第二格越靠后"></label>
  </div>
  <div id="arrangeTrackFields" aria-label="选中的音轨"></div>
  <div id="arrangePreviewText" role="status"></div>
  <ul id="arrangePitchRanges" class="arrange-pitch-ranges" aria-label="各轨编辑结果" hidden></ul>
  <div class="arrange-dialog-footer"><button id="arrangePreview" class="ghost">预览</button><button id="arrangeApply" class="primary">应用</button></div>`;
document.body.append(rangeDialog);
wireDialog(rangeDialog);
$('arrangeClose').onclick=()=>rangeDialog.close();
const rangeAudition=arrangementAuditionControls(rangeDialog);

function arrangementAuditionControls(dialog){
  const controls=document.createElement('div');
  controls.className='pitch-audition arrangement-audition';
  controls.hidden=true;
  controls.setAttribute('aria-label','编排 A/B 试听');
  controls.innerHTML=`
    <button type="button" data-listen>试听</button>
    <button type="button" data-side="A" aria-pressed="false">A 原版</button>
    <button type="button" data-side="B" aria-pressed="false">B 草稿</button>
    <button type="button" data-stop aria-label="停止试听">${icon('stop')}</button>
    <output role="status"></output>`;
  dialog.querySelector('.arrange-dialog-footer').before(controls);
  const listen=controls.querySelector('[data-listen]');
  const buttons=[...controls.querySelectorAll('[data-side]')];
  const stop=controls.querySelector('[data-stop]');
  const output=controls.querySelector('output');
  let draft=null, job=null;
  const current=item=>job===item&&dialog.open&&project?.id===item.scope.project_id&&
    project.revision===item.scope.expected_revision;
  const paint=()=>{
    const ready=job?.state==='complete'&&current(job);
    controls.hidden=!draft||!window.desktopNative;
    listen.disabled=!!job&&job.state!=='complete';
    stop.disabled=!job;
    for(const [index,button] of buttons.entries()){
      button.disabled=!ready;
      button.setAttribute('aria-pressed',String(ready&&audio.auditionId===job.id&&
        audio.auditionVariant===index&&!audio.paused));
    }
  };
  const release=async item=>{
    if(!item?.id||item.released)return;
    item.released=true;
    await audio.stopAudition(item.id).catch(()=>{});
    await api('arrangement_audition',{...item.scope,action:'release',audition_id:item.id}).catch(()=>{});
  };
  const clear=()=>{
    const old=job;job=null;
    clearTimeout(old?.timer);
    output.textContent='';
    paint();
    void release(old);
  };
  const play=async(item,side)=>{
    if(!current(item))return;
    try{await audio.auditionSide(item.id,side)}
    catch(error){if(current(item))output.textContent=error.message}
  };
  const poll=async item=>{
    try{
      if(!current(item)){await release(item);return}
      const status=await api('arrangement_audition',{...item.scope,action:'status',audition_id:item.id});
      if(!current(item)){await release(item);return}
      item.state=status.status;
      if(status.status==='complete'){
        output.textContent='';
        paint();
        await play(item,'B');
      }else if(['running','committing','cancelling'].includes(status.status)){
        item.timer=setTimeout(()=>void poll(item),220);
      }else throw new Error(status.error||'试听未生成');
    }catch(error){
      if(job===item){clear();output.textContent=error.message}
    }
  };
  listen.onclick=async()=>{
    if(!draft||listen.disabled)return;
    if(job){await play(job,'B');return}
    const {scope,edit,preview_id}=draft;
    const item={scope,state:'starting',id:null};
    job=item;
    output.textContent='正在准备 A/B…';
    paint();
    try{
      const snapshot=await api('arrangement_audition',{...scope,action:'prepare',preview_id,edit,
        engine:$('renderEngine').value});
      item.id=snapshot.job_id;
      await poll(item);
    }catch(error){
      if(job===item){clear();output.textContent=error.message}
    }
  };
  for(const button of buttons)button.onclick=()=>{if(job)void play(job,button.dataset.side)};
  stop.onclick=clear;
  for(const event of ['timeupdate','pause'])audio.addEventListener(event,()=>{if(job)paint()});
  const set=value=>{draft=value;clear()};
  dialog.addEventListener('close',()=>set(null));
  return {
    set,
    binding:()=>job?.state==='complete'&&current(job)?{...job.scope,action:'apply',
      preview_id:draft.preview_id,audition_id:job.id}:null
  };
}

const arrangeCanvas=$('arrangeCanvas'),arrangeViewport=$('arrangeViewport');
const arrangeLabelWidth=186,arrangeRow=54;
let arrangeHeader=42;
let arrangementVisible=true,workspaceFocus='arrangement',arrangeZoom=28,arrangeSelection=null,arrangePointer=null;
let arrangeViewEpoch=0,arrangeFocus=null,arrangeContent=null,arrangeLoadedId=null;
let arrangeDraft=null,arrangeVerified=null,arrangePending=false,arrangeFrame=0;
let lastArrangeGesture=0;

const sectionDialog=document.createElement('dialog');
sectionDialog.id='sectionDialog';
sectionDialog.className='arrange-dialog section-dialog';
sectionDialog.setAttribute('aria-labelledby','sectionTitle');
sectionDialog.innerHTML=`
  <div class="dialog-heading"><h2 id="sectionTitle">段落标记</h2><button id="sectionClose" aria-label="关闭">${icon('close')}</button></div>
  <div id="sectionRows"></div>
  <output id="sectionStatus" role="status"></output>
  <div class="arrange-dialog-footer">
    <button id="sectionAdd" class="ghost">${icon('plus')} 新增</button>
    <button id="sectionReset" class="ghost">恢复设计标记</button>
    <button id="sectionSave" class="primary">保存</button>
  </div>`;
document.body.append(sectionDialog);
wireDialog(sectionDialog);
let sectionDraft=null;
let savingSections=false;

function addSectionRow(section){
  const row=document.createElement('div');
  row.className='section-row';
  row.innerHTML='<label>名称<input data-field="name" maxlength="128"></label>'
    +'<label>起点 · 拍<input data-field="start" type="number" min="0" step="any"></label>'
    +'<label>终点 · 拍<input data-field="end" type="number" min="0" step="any"></label>'
    +`<button class="ghost" type="button" aria-label="删除标记">${icon('close')}</button>`;
  for(const input of row.querySelectorAll('input'))input.value=section[input.dataset.field];
  if(typeof section.role==='string')row.dataset.role=section.role;
  row.querySelector('button').onclick=()=>{row.remove();$('sectionAdd').disabled=false};
  $('sectionRows').append(row);
  $('sectionAdd').disabled=$('sectionRows').children.length>=40;
  return row;
}

function showSections(sectionIndex=0){
  if(!project||sectionDialog.open||savingSections)return;
  sectionDraft={project_id:project.id,expected_revision:project.revision};
  $('sectionRows').replaceChildren();
  $('sectionStatus').textContent='';
  $('sectionReset').hidden=project.section_markers==null;
  $('sectionAdd').disabled=false;
  for(const section of arrangementModel().sections)addSectionRow(section);
  sectionDialog.showModal();
  ($('sectionRows').children[sectionIndex]?.querySelector('input')||$('sectionAdd')).focus();
}

async function saveSections(markers){
  if(!sectionDraft||savingSections)return;
  savingSections=true;
  const controls=[...sectionDialog.querySelectorAll('input,button')].filter(control=>!control.disabled);
  for(const control of controls)control.disabled=true;
  try{
    await edit([{op:'set_project',values:{section_markers:markers}}],sectionDraft);
    sectionDialog.close();
  }catch(error){
    $('sectionStatus').textContent=error.message;
  }finally{
    savingSections=false;
    for(const control of controls)control.disabled=false;
  }
}

$('editSections').onclick=()=>showSections();
$('sectionClose').onclick=()=>sectionDialog.close();
$('sectionAdd').onclick=()=>{
  const selection=arrangeSelection;
  const start=selection?.start??cursorBeat;
  const clock=clockFor(),bar=clock.position(start);
  const nextMeter=clock.meters.find(change=>change.beat>start)?.beat??Infinity;
  const end=selection?.end??Math.min(bar.start+bar.meter[0]*4/bar.meter[1],nextMeter,clock.beat(900));
  addSectionRow({name:'',start,end}).querySelector('input').focus();
};
$('sectionReset').onclick=()=>saveSections(null);
$('sectionSave').onclick=()=>{
  const markers=[...$('sectionRows').children].map(row=>{
    const get=field=>row.querySelector(`[data-field="${field}"]`);
    return {name:get('name').value.trim(),start:get('start').valueAsNumber,end:get('end').valueAsNumber,
      ...(row.dataset.role===undefined?{}:{role:row.dataset.role})};
  }).sort((a,b)=>a.start-b.start);
  if(markers.some(({name,start,end},index)=>!name||!Number.isFinite(start)||!Number.isFinite(end)
      ||start<(markers[index-1]?.end??0)||end<=start)){
    $('sectionStatus').textContent='请输入名称和有效拍位，段落不可重叠';
    return;
  }
  void saveSections(markers);
};
sectionDialog.addEventListener('cancel',event=>{if(savingSections)event.preventDefault()});

function arrangementModel(){
  if(arrangeContent?.project===project)return arrangeContent;
  const clock=clockFor();
  const tracks=project.tracks.map(track=>{
    const clips=trackClips(track);
    const low=track.notes.reduce((n,v)=>Math.min(n,v.pitch),127);
    const high=track.notes.reduce((n,v)=>Math.max(n,v.pitch),0);
    return {track,low,high,clips};
  });
  const markers=project.section_markers??project.composition?.sections;
  const sections=(Array.isArray(markers)?markers:[]).filter(s=>Number.isFinite(s?.start)&&Number.isFinite(s.end)&&s.start>=0&&s.end>s.start&&clock.seconds(s.end)<=900);
  let last=16;
  for(const item of tracks){
    for(const clip of item.clips)last=Math.max(last,clipEndBeat(clip));
    for(const note of item.track.notes)last=Math.max(last,note.start+note.duration);
  }
  for(const section of sections)last=Math.max(last,section.end);
  const harmony=project.harmony||[];
  if(harmony.length)last=Math.max(last,harmony.at(-1).beat+1);
  const limit=Math.min(clock.beat(900),Math.ceil(last/4)*4+8);
  arrangeContent={project,tracks,sections,harmony,limit,bars:clock.bars(limit)};
  return arrangeContent;
}

function queueArrangementDraw(){
  if(!arrangementVisible||arrangeFrame)return;
  arrangeFrame=requestAnimationFrame(()=>{arrangeFrame=0;drawArrangement()});
}

function drawArrangement(){
  if(!arrangementVisible||!project)return;
  const model=arrangementModel(),w=arrangeViewport.clientWidth,h=arrangeViewport.clientHeight;
  if(!w||!h)return;
  arrangeHeader=model.harmony.length?60:42;
  $('arrangeExtent').style.width=Math.ceil(arrangeLabelWidth+model.limit*arrangeZoom)+'px';
  $('arrangeExtent').style.height=(arrangeHeader+model.tracks.length*arrangeRow)+'px';
  const sx=arrangeViewport.scrollLeft,sy=arrangeViewport.scrollTop;
  const x=beat=>arrangeLabelWidth+beat*arrangeZoom-sx;
  const y=index=>arrangeHeader+index*arrangeRow-sy;
  arrangeCanvas.style.transform=`translate(${sx}px,${sy}px)`;
  if(typeof placeTrackHeaders==='function')placeTrackHeaders(sx);
  const ctx=canvasSize(arrangeCanvas,w,h);
  ctx.clearRect(0,0,w,h);
  ctx.fillStyle='#0b0b0c';ctx.fillRect(0,0,w,h);
  for(let i=0;i<model.tracks.length;i++){
    const top=y(i);
    if(top+arrangeRow<arrangeHeader||top>h)continue;
    ctx.fillStyle=i%2?'#101012':'#0b0b0c';
    ctx.fillRect(0,top,w,arrangeRow);
    ctx.fillStyle='#202022';ctx.fillRect(0,top+arrangeRow-1,w,1);
  }
  ctx.save();
  ctx.beginPath();ctx.rect(arrangeLabelWidth,arrangeHeader,w-arrangeLabelWidth,h-arrangeHeader);ctx.clip();
  for(const bar of model.bars){
    const left=x(bar.beat);if(left<arrangeLabelWidth||left>w)continue;
    ctx.fillStyle='#232327';ctx.fillRect(left,arrangeHeader,1,h-arrangeHeader);
  }
  const box=arrangePointer?.range||arrangeSelection;
  paintAgentPreview(ctx,audio,x,arrangeHeader,h-arrangeHeader);
  for(let i=0;i<model.tracks.length;i++){
    const {track,clips,low,high}=model.tracks[i],top=y(i);
    if(top+arrangeRow<arrangeHeader||top>h)continue;
    const color=trackColor(track);
    ctx.globalAlpha=track.mute?.38:1;
    for(const clip of clips){
      paintClipWave(ctx,track,clip,x,top+8,38,arrangeLabelWidth,w,queueArrangementDraw);
      if(typeof paintClipSelection==='function')paintClipSelection(ctx,track,clip,x,top);
    }
    if(track.notes.length){
      ctx.fillStyle=color;
      for(const note of track.notes){
        const left=x(note.start),width=Math.max(2,note.duration*arrangeZoom-1);
        if(left>w||left+width<arrangeLabelWidth)continue;
        const pitchOffset=high===low?0:(note.pitch-(low+high)/2)*24/(high-low);
        ctx.fillRect(left,top+28-pitchOffset,width,4);
      }
    }
    ctx.fillStyle='#9c9ca0';
    for(const controller of track.controllers||[]){
      const left=x(controller.beat);
      if(left>=arrangeLabelWidth&&left<w)ctx.fillRect(left,top+47,2,4);
    }
    ctx.globalAlpha=1;
    if(box?.ids.includes(track.id)){
      ctx.fillStyle='#bbbbbb22';
      ctx.fillRect(x(box.start),top+1,x(box.end)-x(box.start),arrangeRow-2);
    }
    for(const region of arrangeFocus?.regions||[]){
      if(region.track_id!==track.id)continue;
      const [start,end]=region.beat_range,left=x(start),width=x(end)-left;
      ctx.fillStyle='#dedede22';ctx.fillRect(left,top+1,width,arrangeRow-2);
      ctx.fillStyle='#dedee1';ctx.fillRect(left,top+1,2,arrangeRow-2);
    }
  }
  if(typeof paintClipDrag==='function')paintClipDrag(ctx,x,y);
  if(typeof arrangeTransportLine==='undefined'){
    const current=x(audio.paused?cursorBeat:secondsBeat(audio.currentTime));
    ctx.fillStyle='#cfcfd2';ctx.fillRect(current,arrangeHeader,1,h-arrangeHeader);
  }else updateArrangePlayhead();
  ctx.restore();
  ctx.fillStyle='#151517';ctx.fillRect(0,0,arrangeLabelWidth,h);
  ctx.font='12px Studio Inter, Studio Han, sans-serif';
  for(let i=0;i<model.tracks.length;i++){
    const {track}=model.tracks[i],top=y(i);
    if(top+arrangeRow<arrangeHeader||top>h)continue;
    if(track.id===trackId){ctx.fillStyle='#333335';ctx.fillRect(1,top+3,arrangeLabelWidth-2,arrangeRow-6)}
    ctx.fillStyle='#aaaab0';ctx.fillText(track.name,16,top+26,arrangeLabelWidth-28);
    ctx.fillStyle='#adadb1';ctx.font='10px Studio Inter, Studio Han, sans-serif';
    ctx.fillText(isAudioTrack(track)?'伴奏':track.instrument?'乐器':'歌声',16,top+42);
    ctx.font='12px Studio Inter, Studio Han, sans-serif';
  }
  ctx.fillStyle='#1a1a1b';ctx.fillRect(0,0,w,arrangeHeader);
  ctx.save();ctx.beginPath();ctx.rect(arrangeLabelWidth,0,w-arrangeLabelWidth,arrangeHeader);ctx.clip();
  ctx.font='10px Studio Inter, Studio Han, sans-serif';
  for(const bar of model.bars){
    const left=x(bar.beat);if(left<arrangeLabelWidth||left>w)continue;
    ctx.fillStyle=bar.change?'#bdbdc2':'#aaaab0';
    ctx.fillText(bar.bar+(bar.change?' · '+bar.meter.join('/'):' '),left+5,16);
  }
  for(const section of model.sections){
    const left=Math.max(arrangeLabelWidth,x(section.start)),right=Math.min(w,x(section.end));
    if(right<=left)continue;
    ctx.fillStyle='#29292d';ctx.fillRect(left+1,22,right-left-2,16);
    ctx.fillStyle='#bcbcc0';ctx.fillText(section.name,left+8,34,Math.max(0,right-left-14));
  }
  for(let i=0;i<model.harmony.length;i++){
    const point=model.harmony[i],end=model.harmony[i+1]?.beat??model.limit;
    const left=Math.max(arrangeLabelWidth,x(point.beat)),right=Math.min(w,x(end));
    if(right<=left)continue;
    ctx.fillStyle='#28282a';ctx.fillRect(left+1,41,right-left-2,17);
    ctx.fillStyle='#d0d0d4';ctx.fillText(point.symbol,left+6,53,Math.max(0,right-left-9));
  }
  ctx.restore();
  ctx.fillStyle='#a0a0a6';ctx.fillText('轨道',14,25);
  if(model.harmony.length)ctx.fillText('和弦',14,53);
  ctx.fillStyle='#2b2b30';ctx.fillRect(0,arrangeHeader-1,w,1);
  if(arrangeFocus&&followAgent){
    const row=project.tracks.findIndex(track=>track.id===arrangeFocus.id);
    const rowOnly=arrangeFocus.start===null;
    const px=rowOnly?38:x(arrangeFocus.start),py=y(row)+12;
    if(row>=0&&(rowOnly||px>=arrangeLabelWidth-1)&&px<w&&py>=arrangeHeader&&py<h){
      const rect=arrangeCanvas.getBoundingClientRect();
      showCursor(rect.left+px,rect.top+py,null);
    }else agentCursor.classList.remove('visible');
  }
}

function arrangeSelectionIds(){
  const range=arrangeSelection;
  if(!range||!project)return [];
  const tracks=new Set(range.ids);
  return project.tracks.filter(track=>tracks.has(track.id)).flatMap(track=>track.notes
    .filter(note=>note.start>=range.start-1e-8&&note.start+note.duration<=range.end+1e-8)
    .map(note=>note.id));
}

function arrangeRangeTouchesNotes(pitchedOnly=false){
  const selection=arrangeSelection;
  return !!selection&&!!project?.tracks.some(track=>
    selection.ids.includes(track.id)&&(!pitchedOnly||!track.instrument?.drums)&&track.notes.some(note=>
      note.start<selection.end&&note.start+note.duration>selection.start
    ));
}
function paintArrangeRangeLabel(){
  const selection=arrangeSelection;
  $('arrangeSpan').textContent=selection?`${selection.start.toFixed(2)}–${selection.end.toFixed(2)} 拍 · ${selection.ids.length} 轨`:'';
}
function updateArrangeSelection(selection){
  arrangeSelection=selection;
  paintArrangeRangeLabel();
  for(const id of ['arrangeCopy','arrangeMove','arrangeClear'])$(id).disabled=!selection?.ids.length;
  $('arrangeTranspose').disabled=!arrangeRangeTouchesNotes(true);
  $('arrangeQuantize').disabled=!arrangeRangeTouchesNotes();
  queueArrangementDraw();
  publishSelection();
}

async function switchArrangement(show){
  workspaceFocus=show?'arrangement':'editor';
  if(show){arrangeViewport.focus({preventScroll:true});queueArrangementDraw()}
  else{await setDetailsVisible(true);draw()}
  publishSelection();
}

async function setDetailsVisible(show){
  const epoch=++arrangeViewEpoch;
  if(show){
    const entering=scorePanel.hidden;
    scorePanel.getAnimations().forEach(animation=>animation.cancel());
    scorePanel.hidden=false;
    if(entering)animateElement(scorePanel,[{opacity:0,transform:'translateY(9px)'},{opacity:1,transform:'translateY(0)'}],'panelIn');
    draw();
  }else if(!scorePanel.hidden){
    await animateElement(scorePanel,[{opacity:1},{opacity:0,transform:'translateY(9px)'}],'panelOut').finished.catch(()=>{});
    if(epoch!==arrangeViewEpoch)return;
    scorePanel.hidden=true;
    workspaceFocus='arrangement';
    arrangeViewport.focus({preventScroll:true});
  }
  viewButton.setAttribute('aria-pressed',String(show));
  queueArrangementDraw();
}

function arrangePoint(event){
  const box=arrangeCanvas.getBoundingClientRect();
  const x=event.clientX-box.left,y=event.clientY-box.top;
  return {x,y,beat:Math.max(0,(x-arrangeLabelWidth+arrangeViewport.scrollLeft)/arrangeZoom),
    row:Math.floor((y-arrangeHeader+arrangeViewport.scrollTop)/arrangeRow)};
}

function arrangementSeek(beat){
  cursorBeat=beat;
  if(window.desktopNative||audio.readyState>0)audio.currentTime=Math.min(audio.duration,beatSeconds(beat));
  queueArrangementDraw();
}

arrangeCanvas.addEventListener('dblclick',event=>{
  if(!project||arrangePending)return;
  const point=arrangePoint(event);
  if(point.x<arrangeLabelWidth||point.y<22||point.y>=38)return;
  const index=arrangementModel().sections.findIndex(section=>point.beat>=section.start&&point.beat<section.end);
  if(index>=0)showSections(index);
});

arrangeCanvas.onpointerdown=event=>{
  if(!project||event.button!==0||arrangePending)return;
  const point=arrangePoint(event);
  if(point.x<arrangeLabelWidth)return;
  if(point.y<arrangeHeader){
    const section=point.y>=22&&point.y<38&&arrangementModel().sections.find(s=>point.beat>=s.start&&point.beat<s.end);
    if(section){
      workspaceFocus='arrangement';
      arrangeViewport.focus({preventScroll:true});
      const range={start:section.start,end:section.end,ids:project.tracks.map(t=>t.id)};
      updateArrangeSelection(range);
      if(typeof selectClipsInRange==='function')selectClipsInRange(range);
    }
    arrangementSeek(section?section.start:snap(point.beat));
    return;
  }
  workspaceFocus='arrangement';
  arrangeViewport.focus({preventScroll:true});
  arrangePointer={start:point,range:null,id:event.pointerId};
  if(arrangeCanvas.hasPointerCapture(event.pointerId)===false){
    try{arrangeCanvas.setPointerCapture(event.pointerId)}catch(error){if(error.name!=='NotFoundError')throw error}
  }
};
arrangeCanvas.onpointermove=event=>{
  if(!arrangePointer||arrangePointer.id!==event.pointerId)return;
  const point=arrangePoint(event),anchor=arrangePointer.start;
  if(Math.hypot(point.x-anchor.x,point.y-anchor.y)<4)return;
  const step=Number($('quantize').value);
  const start=Math.floor(Math.min(point.beat,anchor.beat)/step)*step;
  const end=Math.max(start+step,Math.ceil(Math.max(point.beat,anchor.beat)/step)*step);
  const tracks=project.tracks.slice(Math.max(0,Math.min(point.row,anchor.row)),Math.max(point.row,anchor.row)+1);
  arrangePointer.range={start,end,ids:tracks.map(t=>t.id)};
  queueArrangementDraw();
};
arrangeCanvas.onpointerup=event=>{
  if(!arrangePointer||arrangePointer.id!==event.pointerId)return;
  const {range,start}=arrangePointer;arrangePointer=null;
  if(range?.ids.length){updateArrangeSelection(range);if(typeof selectClipsInRange==='function')selectClipsInRange(range)}
  else{
    updateArrangeSelection(null);
    const t=project.tracks[start.row];if(t){trackId=t.id;chooseGroup([]);refresh()}
    arrangementSeek(snap(start.beat));
  }
};
arrangeCanvas.onpointercancel=()=>{arrangePointer=null;queueArrangementDraw()};
arrangeCanvas.ondblclick=async event=>{
  const point=arrangePoint(event),track=project?.tracks[point.row];
  if(arrangeHeader===60&&point.y>=41&&point.y<60&&point.x>=arrangeLabelWidth){
    await openHarmony(point.beat);return;
  }
  if(!track||point.y<arrangeHeader)return;
  trackId=track.id;chooseGroup([]);refresh();
  await switchArrangement(false);
  const view=$('rollViewport');view.scrollLeft=Math.max(0,beatX(point.beat)-view.clientWidth/3);
  const nearest=track.notes.find(note=>note.start+note.duration>=point.beat);
  if(nearest)view.scrollTop=Math.max(0,pitchY(nearest.pitch)-view.clientHeight/2);
};

function arrangeResize(factor){
  const before=arrangeZoom;
  arrangeZoom=Math.min(128,Math.max(6,arrangeZoom*factor));
  if(before===arrangeZoom)return;
  const centre=(arrangeViewport.scrollLeft+(arrangeViewport.clientWidth-arrangeLabelWidth)/2)/before;
  drawArrangement();
  arrangeViewport.scrollLeft=Math.max(0,centre*arrangeZoom-(arrangeViewport.clientWidth-arrangeLabelWidth)/2);
}
$('arrangeZoomIn').onclick=()=>arrangeResize(1.28);
$('arrangeZoomOut').onclick=()=>arrangeResize(1/1.28);
arrangeViewport.addEventListener('scroll',queueArrangementDraw,{passive:true});
for(const kind of ['wheel','pointerdown'])arrangeViewport.addEventListener(kind,event=>{if(event.isTrusted)lastArrangeGesture=performance.now()},{passive:true,capture:true});
new ResizeObserver(queueArrangementDraw).observe(arrangeViewport);

function openArrangeEdit(action,shift=1){
  if(!project||rangeDialog.open)return;
  const current=arrangeSelection||{start:cursorBeat,end:cursorBeat+barLength(project.meter),ids:[trackId]};
  arrangeDraft={action,id:project.id,revision:project.revision};
  arrangeVerified=null;
  rangeAudition.set(null);
  $('arrangeTitle').textContent={copy:'复制片段',move:'移动片段',clear:'清除片段',select:'选择范围',transpose:'多轨移调',quantize:'多轨量化'}[action];
  $('arrangeFrom').value=current.start;$('arrangeTo').value=current.end;
  $('arrangeTarget').value=current.end;
  $('arrangeTargetLabel').hidden=action!=='copy'&&action!=='move';
  $('arrangeShiftLabel').hidden=action!=='transpose';
  $('arrangeShift').value=shift;
  $('arrangeTimingFields').hidden=action!=='quantize';
  if(action==='quantize')$('arrangeGrid').value=$('quantize').value;
  $('arrangeTrackFields').innerHTML=project.tracks.map(t=>
    '<label><input type="checkbox" value="'+esc(t.id)+'" '+(current.ids.includes(t.id)?'checked':'')+'><span>'+esc(t.name)+'</span></label>'
  ).join('');
  $('arrangePreviewText').textContent='';
  $('arrangePitchRanges').hidden=true;
  $('arrangePreview').hidden=action==='select';
  $('arrangeApply').disabled=action!=='select';
  rangeDialog.showModal();
}

function readArrangeDraft(){
  const fields=['arrangeFrom','arrangeTo'];
  if(!$('arrangeTargetLabel').hidden)fields.push('arrangeTarget');
  const values=fields.map(id=>$(id).valueAsNumber);
  const [start,end,target]=values;
  const track_ids=[...$('arrangeTrackFields').querySelectorAll('input:checked')].map(element=>element.value);
  if(values.some(v=>!Number.isFinite(v)||v<0)||end<=start||!track_ids.length||beatSeconds(end)>900)
    throw new Error('请选择音轨，并输入有效的拍范围');
  const action=arrangeDraft.action;
  const request={project_id:arrangeDraft.id,expected_revision:arrangeDraft.revision,action,track_ids,source:[start,end]};
  if(action==='transpose'){
    const shift=$('arrangeShift').valueAsNumber;
    if(!Number.isInteger(shift)||shift===0||Math.abs(shift)>127)throw new Error('输入 ±1 至 ±127 个半音');
    request.semitones=shift;
  }else if(action==='quantize'){
    const step=Number($('arrangeGrid').value);
    const strength=$('arrangeStrength').valueAsNumber/100,swing=$('arrangeSwing').valueAsNumber/100;
    if(!(step>0&&strength>=0.01&&strength<=1&&swing>=0.5&&swing<=0.75))
      throw new Error('强度须为 1% 至 100%，摇摆须为 50% 至 75%');
    request.quantize={step,strength,swing};
  }else if(action!=='select'){
    request.slice_audio=true;
  }
  if(target!==undefined)request.target_start=target;
  return request;
}
rangeDialog.addEventListener('input',()=>{
  arrangeVerified=null;
  rangeAudition.set(null);
  if(arrangeDraft?.action!=='select')$('arrangeApply').disabled=true;
  $('arrangePreviewText').textContent='';
  $('arrangePitchRanges').hidden=true;
});

function arrangementChangeCount(result){
  return [result.note_count&&result.note_count+' 个音符',result.clip_count&&result.clip_count+' 个音频片段'].filter(Boolean).join(' · ');
}

function arrangementChangeSummary(result){
  const count=arrangementChangeCount(result)+' · '+result.tracks.length+' 轨';
  if(result.action==='quantize'){
    const audioCount=result.warnings.filter(row=>row.kind==='quantize_audio_stays').reduce((total,row)=>total+row.count,0);
    return count+' · 最大 '+result.max_shift_ms.toFixed(1)+' ms'+(audioCount?' · '+audioCount+' 段音频原位':'');
  }
  if(result.action!=='transpose')return count;
  const shift=(result.semitones>0?'+':'')+result.semitones;
  const skipped=result.warnings.filter(row=>row.kind==='transpose_skipped').length;
  return shift+' 半音 · '+count+(skipped?' · '+skipped+' 轨原音':'');
}

function paintArrangeTrackChanges(result){
  const list=$('arrangePitchRanges');
  const rows=[];
  const addRow=(label,detail)=>{
    const row=document.createElement('li');
    const name=document.createElement('span'),status=document.createElement('span');
    name.textContent=name.title=label;
    status.textContent=detail;
    row.append(name,status);
    rows.push(row);
  };
  const octaves=([low,high])=>low===high?noteName(low):noteName(low)+'–'+noteName(high);
  for(const track of result.tracks){
    if(result.action==='quantize')addRow(track.name,
      track.count+' 个音符 · 最大 '+track.max_shift_ms.toFixed(1)+' ms');
    if(result.action==='transpose')addRow(track.name,
      octaves(track.pitch_range.before)+' → '+octaves(track.pitch_range.after));
    for(const change of track.lyric_changes||[]){
      const partial=change.affected_note_count<change.line_note_count;
      const count=change.affected_note_count+'/'+change.line_note_count+' 音';
      const detail={
        copied:'复制原词',
        not_copied:'复制 '+count+' · 原词留在原句',
        moved:partial?'移动 '+count+' · 原词保留':'原词随句移动',
        notes_removed:'原词保留 · 缺失 '+count,
        alignment_changed:'原词待核对'
      }[change.kind];
      const review=change.state_after==='alignment_changed'&&
        ['copied','moved'].includes(change.kind)?' · 核对落字':'';
      addRow(track.name+' · '+(change.text||'原词'),detail+review);
    }
  }
  list.replaceChildren(...rows);
  list.hidden=!rows.length;
}

$('arrangePreview').onclick=async()=>{
  const draft=arrangeDraft;
  try{
    await editChain.catch(()=>{});
    const request=readArrangeDraft(),key=JSON.stringify(request);
    $('arrangePreview').disabled=true;
    const result=await api('arrangement_edit',request);
    if(!rangeDialog.open||draft!==arrangeDraft||JSON.stringify(readArrangeDraft())!==key)return;
    arrangeVerified={request,preview_id:result.preview_id};
    const {project_id,expected_revision,...edit}=request;
    rangeAudition.set({scope:{project_id,expected_revision},preview_id:result.preview_id,edit:{kind:'range',edit}});
    const controls=result.warnings.filter(row=>row.kind==='controllers_stay').reduce((count,row)=>count+row.count,0);
    const automation=result.warnings.some(row=>row.kind==='gain_automation_stays');
    $('arrangePreviewText').textContent=arrangementChangeSummary(result)+
      (result.cut_count?' · 按边界截取 '+result.cut_count+' 段音频':'')+
      (controls?' · '+controls+' 个 MIDI 控制点留在原位':'')+(automation?' · 音量自动化留在原位':'');
    paintArrangeTrackChanges(result);
    $('arrangeApply').disabled=false;
  }catch(error){$('arrangePreviewText').textContent=error.message}
  finally{$('arrangePreview').disabled=false}
};

$('arrangeApply').onclick=async()=>{
  if(arrangePending)return;
  try{
    const request=readArrangeDraft();
    if(request.action==='select'){
      const range={start:request.source[0],end:request.source[1],ids:request.track_ids};
      workspaceFocus='arrangement';
      updateArrangeSelection(range);
      if(typeof selectClipsInRange==='function')selectClipsInRange(range);
    }else{
      if(!arrangeVerified||JSON.stringify(request)!==JSON.stringify(arrangeVerified.request))throw new Error('请预览当前范围');
      arrangePending=true;
      $('arrangeApply').disabled=true;
      const preview_id=arrangeVerified.preview_id;
      const listening=rangeAudition.binding();
      const commit=editChain.catch(()=>{}).then(()=>listening
        ? api('arrangement_audition',listening)
        : api('arrangement_edit',{...request,preview_id}));
      editChain=commit;
      const result=await commit;
      if(project?.id===request.project_id){
        accept(result.project);
        const bounds=result.destination||result.source;
        const range={start:bounds[0],end:bounds[1],ids:request.track_ids};
        updateArrangeSelection(range);
        if(typeof selectClipsInRange==='function')selectClipsInRange(range);
      }
      const operation={transpose:'已移调',quantize:'已量化'}[request.action];
      toast(operation?operation+' '+arrangementChangeSummary(result):'已编辑 '+arrangementChangeCount(result));
      await refreshList();
    }
    rangeDialog.close();
  }catch(error){$('arrangePreviewText').textContent=error.message}
  finally{arrangePending=false}
};
for(const [id,action] of [['arrangeRange','select'],['arrangeCopy','copy'],['arrangeMove','move'],['arrangeClear','clear'],['arrangeTranspose','transpose'],['arrangeQuantize','quantize']]){
  $(id).onclick=()=>openArrangeEdit(action);
}

const refreshBeforeArrangement=refresh;
refresh=function(external=false){
  refreshBeforeArrangement(external);
  if(project&&project.id!==arrangeLoadedId){
    arrangeLoadedId=project.id;arrangeFocus=null;arrangePointer=null;
    updateArrangeSelection(null);arrangeViewport.scrollTo(0,0);
  }
  $('arrangeTranspose').disabled=!arrangeRangeTouchesNotes(true);
  $('arrangeQuantize').disabled=!arrangeRangeTouchesNotes();
  queueArrangementDraw();
};

async function focusArrangement(event,generation){
  const regions=event.regions||(Array.isArray(event.beat_range)?[{track_id:event.track_id||trackId,beat_range:event.beat_range}]:[]);
  if(!project||generation!==focusGeneration)return;
  const target=regions.find(region=>project.tracks.some(track=>track.id===region.track_id));
  const id=target?.track_id||event.track_id;
  const row=project.tracks.findIndex(track=>track.id===id);
  if(row<0)return;
  const [start,end]=target?.beat_range||[null,null];
  arrangeFocus={id,start,end,regions};
  if(followAgent&&performance.now()-lastArrangeGesture>850){
    const view=arrangeViewport,px=start*arrangeZoom,py=row*arrangeRow;
    const xInside=start===null||(px>=view.scrollLeft&&px<view.scrollLeft+view.clientWidth-arrangeLabelWidth-14);
    const yInside=py>=view.scrollTop&&py+arrangeRow<=view.scrollTop+view.clientHeight-arrangeHeader;
    if(!xInside||!yInside)view.scrollTo(xInside?view.scrollLeft:Math.max(0,px-(view.clientWidth-arrangeLabelWidth)/3),yInside?view.scrollTop:Math.max(0,py-arrangeRow));
  }
  drawArrangement();
  const focus=arrangeFocus;
  setTimeout(()=>{if(arrangeFocus===focus){arrangeFocus=null;agentCursor.classList.remove('visible');queueArrangementDraw()}},4200);
}

updateArrangeSelection(null);
