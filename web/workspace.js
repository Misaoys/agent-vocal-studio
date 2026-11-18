'use strict';

const studioMain=document.querySelector('main');
const studioWorkspace=document.createElement('div');
studioWorkspace.className='studio-workspace';
const divider=document.createElement('div');
divider.className='workspace-divider';
divider.tabIndex=0;
divider.setAttribute('role','separator');
divider.setAttribute('aria-label','调整编排和编辑器高度');
divider.setAttribute('aria-orientation','horizontal');
arrangementPanel.before(studioWorkspace);
studioWorkspace.append(arrangementPanel,divider,scorePanel);
studioWorkspace.style.setProperty('--details-height','42%');

let splitStart=null;
function resizeDetail(percent){
  const value=Math.max(30,Math.min(65,percent));
  studioWorkspace.style.setProperty('--details-height',value+'%');
  divider.setAttribute('aria-valuenow',String(Math.round(value)));
  queueArrangementDraw();
}
divider.onpointerdown=event=>{
  if(event.button!==0)return;
  splitStart={id:event.pointerId,top:studioWorkspace.getBoundingClientRect().top,height:studioWorkspace.clientHeight};
  divider.setPointerCapture(event.pointerId);
};
divider.onpointermove=event=>{
  if(splitStart?.id!==event.pointerId)return;
  resizeDetail(100*(1-(event.clientY-splitStart.top)/splitStart.height));
};
divider.onpointerup=divider.onpointercancel=()=>{splitStart=null};
divider.onkeydown=event=>{
  if(!['ArrowUp','ArrowDown'].includes(event.key))return;
  event.preventDefault();
  resizeDetail(Number(divider.getAttribute('aria-valuenow')||42)+(event.key==='ArrowUp'?5:-5));
};

const tracksRoot=$('tracks');
tracksRoot.classList.add('arrange-track-headers');
arrangeViewport.append(tracksRoot);
const addTrack=$('addTrack');
arrangementPanel.querySelector('.arrange-toolbar').prepend(addTrack);
addTrack.title='添加音轨';
const addTracks=arrangementPanel.querySelector('.arrange-toolbar');
const trackChoices=document.createElement('div');
trackChoices.className='track-choices';
trackChoices.innerHTML=`<button id="addAudioLane" title="导入音频到新音轨" aria-label="添加音频轨">${icon('wave')}</button>`;
addTrack.after(trackChoices);
$('addAudioLane').onclick=()=>{$('importBtn').click()};
tracksRoot.style.setProperty('--track-row',arrangeRow+'px');
tracksRoot.style.width=arrangeLabelWidth+'px';
tracksRoot.style.top=arrangeHeader+'px';
function placeTrackHeaders(scrollLeft){
  tracksRoot.style.top=arrangeHeader+'px';
  tracksRoot.style.transform=`translateX(${scrollLeft}px)`;
  tracksRoot.style.clipPath=`inset(${arrangeViewport.scrollTop}px 0 0 0)`;
}

const trackOrderMarker=document.createElement('div');
trackOrderMarker.className='track-order-marker';
trackOrderMarker.setAttribute('aria-hidden','true');
let trackOrderDrag=null,trackOrderFrame=0,trackOrderSaving=false;
let duplicatingTrack=false;

async function duplicateEditorTrack(id=trackId){
  const original=project;
  const from=original?.tracks.findIndex(track=>track.id===id)??-1;
  if(from<0||duplicatingTrack||trackOrderDrag)return;
  duplicatingTrack=true;
  try{
    const saved=await edit([{op:'duplicate_track',track_id:id}],{
      project_id:original.id,expected_revision:original.revision
    });
    if(project?.id!==saved?.id)return;
    const copy=project.tracks.find(track=>track.id===saved.tracks[from+1].id);
    if(!copy)return;
    trackId=copy.id;
    chooseGroup([]);
    updateArrangeSelection(null);
    selectAudioClip(copy,null);
    revealArrangementTrack(copy.id);
    toast('已复制音轨 · 副本已静音');
  }finally{duplicatingTrack=false}
}

tracksRoot.addEventListener('click',event=>{
  const duplicate=event.target.closest('[data-duplicate-track]');
  if(duplicate)void duplicateEditorTrack(duplicate.dataset.duplicateTrack).catch(()=>{});
});

async function commitTrackOrder(track,before,guard){
  if(trackOrderSaving)return;
  trackOrderSaving=true;
  const focusedTrack=tracksRoot.contains(document.activeElement)?document.activeElement.closest('[data-track]')?.dataset.track:null;
  try{
    await edit([{op:'move_track',track_id:track,before_track_id:before}],guard);
    if(focusedTrack&&document.activeElement===document.body)
      tracksRoot.querySelector(`[data-track="${CSS.escape(focusedTrack)}"]`)?.focus({preventScroll:true});
  }finally{trackOrderSaving=false}
}

async function moveActiveTrack(direction){
  const index=project?.tracks.findIndex(track=>track.id===trackId)??-1;
  if(index<0||!project.tracks[index+direction]||trackOrderDrag)return;
  const before=project.tracks[index+(direction<0?-1:2)]?.id??null;
  const id=trackId;
  await commitTrackOrder(id,before,{project_id:project.id,expected_revision:project.revision});
  if(tr()?.id===id)revealArrangementTrack(id);
}

function revealArrangementTrack(id){
  const position=project.tracks.findIndex(track=>track.id===id)*arrangeRow;
  const visible=arrangeViewport.clientHeight-arrangeHeader;
  const top=arrangeViewport.scrollTop;
  if(position<top)arrangeViewport.scrollTop=position;
  else if(position+arrangeRow>top+visible)arrangeViewport.scrollTop=position+arrangeRow-visible;
}

function stopTrackOrderDrag(){
  const drag=trackOrderDrag;
  trackOrderDrag=null;
  if(trackOrderFrame)cancelAnimationFrame(trackOrderFrame);
  trackOrderFrame=0;
  trackOrderMarker.remove();
  drag?.card.classList.remove('track-order-moving');
  if(drag&&tracksRoot.hasPointerCapture(drag.pointer))tracksRoot.releasePointerCapture(drag.pointer);
  return drag;
}

function drawTrackOrderDrop(scroll){
  const drag=trackOrderDrag;
  if(!drag||Math.abs(drag.y-drag.origin)<4&&!drag.moving)return;
  if(project?.id!==drag.project_id||project.revision!==drag.expected_revision){
    stopTrackOrderDrag();
    return;
  }
  drag.moving=true;
  const rect=arrangeViewport.getBoundingClientRect();
  const top=rect.top+arrangeHeader,bottom=rect.top+arrangeViewport.clientHeight;
  const beforeScroll=arrangeViewport.scrollTop;
  if(scroll){
    const edge=26;
    const dy=drag.y<top+edge?-Math.min(12,(top+edge-drag.y)*.45):
      drag.y>bottom-edge?Math.min(12,(drag.y-bottom+edge)*.45):0;
    if(dy)arrangeViewport.scrollTop+=dy;
  }
  const position=Math.min(bottom,Math.max(top,drag.y))-top+arrangeViewport.scrollTop;
  drag.boundary=Math.max(0,Math.min(project.tracks.length,Math.round(position/arrangeRow)));
  drag.card.classList.add('track-order-moving');
  if(!trackOrderMarker.isConnected)tracksRoot.append(trackOrderMarker);
  trackOrderMarker.hidden=drag.boundary===drag.from||drag.boundary===drag.from+1;
  trackOrderMarker.style.top=(drag.boundary*arrangeRow-1)+'px';
  if(scroll&&arrangeViewport.scrollTop!==beforeScroll){
    trackOrderFrame=requestAnimationFrame(()=>{trackOrderFrame=0;drawTrackOrderDrop(true)});
  }
}

tracksRoot.addEventListener('pointerdown',event=>{
  const grip=event.target.closest('[data-track-order]');
  if(!grip||event.button!==0||!project||trackOrderDrag||trackOrderSaving||project.tracks.length<2)return;
  const card=grip.closest('[data-track]');
  const from=project.tracks.findIndex(track=>track.id===card.dataset.track);
  if(from<0)return;
  event.preventDefault();
  trackOrderDrag={pointer:event.pointerId,card,from,y:event.clientY,origin:event.clientY,
    project_id:project.id,expected_revision:project.revision,moving:false,boundary:from};
  lastArrangeGesture=performance.now();
  try{tracksRoot.setPointerCapture(event.pointerId)}catch(error){if(error.name!=='NotFoundError')throw error}
});
tracksRoot.addEventListener('pointermove',event=>{
  if(trackOrderDrag?.pointer!==event.pointerId)return;
  trackOrderDrag.y=event.clientY;
  lastArrangeGesture=performance.now();
  if(!trackOrderFrame)drawTrackOrderDrop(true);
});
tracksRoot.addEventListener('pointerup',event=>{
  if(trackOrderDrag?.pointer!==event.pointerId)return;
  trackOrderDrag.y=event.clientY;
  drawTrackOrderDrop(false);
  const drag=stopTrackOrderDrag();
  if(!drag?.moving||drag.boundary===drag.from||drag.boundary===drag.from+1)return;
  const before=project.tracks[drag.boundary]?.id??null;
  void commitTrackOrder(drag.card.dataset.track,before,drag).catch(()=>{});
});
for(const type of ['pointercancel','lostpointercapture'])tracksRoot.addEventListener(type,event=>{
  if(trackOrderDrag?.pointer===event.pointerId)stopTrackOrderDrag();
});
document.addEventListener('keydown',event=>{
  if(event.key==='Escape'&&trackOrderDrag){
    event.preventDefault();
    stopTrackOrderDrag();
  }
});

const drawer=document.createElement('dialog');
drawer.className='project-drawer';
drawer.setAttribute('aria-label','工程');
const projectsAside=document.querySelector('.sidebar');
projectsAside.querySelector('.sidebar-divider')?.remove();
projectsAside.querySelectorAll('.section-label')[1]?.remove();
projectsAside.querySelector('.rust-tag')?.remove();
projectsAside.querySelector('.brand').remove();
projectsAside.querySelector('.section-label').firstChild.textContent='工程 ';
const drawerClose=document.createElement('button');
drawerClose.setAttribute('aria-label','关闭工程列表');
drawerClose.innerHTML=icon('close');
projectsAside.querySelector('.section-label').append(drawerClose);
drawerClose.onclick=()=>drawer.close();
drawer.append(projectsAside);
document.body.append(drawer);
wireDialog(drawer);
const projectButton=document.createElement('button');
projectButton.id='openProjects';
projectButton.setAttribute('aria-label','工程列表');
projectButton.title='工程列表';
projectButton.innerHTML=icon('folder');
studioMain.querySelector('.topbar').prepend(projectButton);
labelButton('projectPrev','left');
labelButton('projectNext','right');
projectButton.onclick=()=>{
  drawer.showModal();
  $('projectSearch').focus({preventScroll:true});
  void refreshList().catch(()=>{});
};
projectsAside.addEventListener('click',event=>{if(event.target.closest('.project-item'))drawer.close()});
studioMain.querySelector('.breadcrumb').firstChild.textContent='';
studioMain.querySelector('.breadcrumb span').remove();

studioMain.querySelector('.statusbar').append(activityBar);

const inspectorToggle=document.createElement('button');
inspectorToggle.id='inspectorToggle';
inspectorToggle.title='音符与音轨属性';
inspectorToggle.setAttribute('aria-pressed','true');
inspectorToggle.innerHTML=icon('plan');
scorePanel.querySelector('.editor-toolbar').append(inspectorToggle);
const inspector=scorePanel.querySelector('.inspector');
inspectorToggle.onclick=async()=>{
  if(inspector.hidden){inspector.hidden=false;animateElement(inspector,[{opacity:0,transform:'translateX(9px)'},{opacity:1,transform:'translateX(0)'}],'panelIn')}
  else{await animateElement(inspector,[{opacity:1},{opacity:0,transform:'translateX(9px)'}],'panelOut').finished.catch(()=>{});inspector.hidden=true}
  inspectorToggle.setAttribute('aria-pressed',String(!inspector.hidden));
  draw();
};

const audioDetail=document.createElement('section');
audioDetail.className='workspace-audio-detail';
audioDetail.hidden=true;
audioDetail.innerHTML=`<div class="wave-detail"><canvas id="selectedWaveform" aria-label="已选音频波形"></canvas><div id="clipGroupList"></div></div>
  <div id="inlineClipFields" class="workspace-audio-controls">
    <label>增益 · dB<input id="inlineClipGain" type="number" min="-60" max="12" step="0.1"></label>
    <label>淡入 · ms<input id="inlineClipIn" type="number" min="0" step="1"></label>
    <label>淡出 · ms<input id="inlineClipOut" type="number" min="0" step="1"></label>
    <button id="inlineClipApply" class="primary">应用</button>
  </div>
  <div id="inlineGroupFields" class="workspace-audio-controls" hidden>
    <label>统一增减 · dB<input id="inlineGroupGain" type="number" min="-72" max="72" step="0.5" value="0"></label>
    <button id="inlineGroupApply" class="primary">应用</button>
  </div>`;
scorePanel.append(audioDetail);
let detailKey='';
let groupTimeline=null,groupWaveFrame=0;
const groupList=$('clipGroupList');
groupList.setAttribute('aria-label','选中音频轨的对齐波形');

function updateGroupPlayhead(){
  if(!groupTimeline||groupList.hidden)return;
  const {start,end,indicator}=groupTimeline;
  const beat=audio.paused?cursorBeat:secondsBeat(audio.currentTime);
  indicator.hidden=beat<start||beat>end;
  indicator.style.left=((beat-start)/(end-start)*100)+'%';
}

function drawAudioGroup(){
  groupWaveFrame=0;
  if(!groupTimeline||groupList.hidden||scorePanel.hidden)return;
  const {start,end,rows,ticks}=groupTimeline;
  const viewport=groupList.parentElement;
  const first=viewport.scrollTop,last=first+viewport.clientHeight;
  for(const {track,clips,canvas,row} of rows){
    if(row.offsetTop+row.offsetHeight<first||row.offsetTop>last)continue;
    const width=canvas.clientWidth,height=canvas.clientHeight;
    if(!width||!height)continue;
    const c=canvasSize(canvas,width,height);
    const x=beat=>(beat-start)/(end-start)*width;
    c.fillStyle='#151517';c.fillRect(0,0,width,height);
    c.fillStyle='#343436';
    for(const beat of ticks)c.fillRect(x(beat),0,1,height);
    for(const clip of clips)paintClipWave(c,track,clip,x,4,height-8,0,width,queueAudioGroup);
  }
  updateGroupPlayhead();
}

function queueAudioGroup(){
  if(!groupWaveFrame&&!groupList.hidden&&groupTimeline)groupWaveFrame=requestAnimationFrame(drawAudioGroup);
}

function renderAudioGroup(group){
  groupTimeline=null;
  groupList.replaceChildren();
  if(!group.length)return;
  const tracks=new Map();
  let start=Infinity,end=0;
  for(const {track,clip} of group){
    start=Math.min(start,clip.start);
    end=Math.max(end,clipEndBeat(clip));
    if(!tracks.has(track.id))tracks.set(track.id,{track,clips:[]});
    tracks.get(track.id).clips.push(clip);
  }
  start=Math.max(0,Math.floor(start));
  end=Math.max(start+1,Math.ceil(end));
  const step=2**Math.ceil(Math.log2(Math.max(1,(end-start)/8)));
  const ticks=[];
  for(let beat=start;beat<=end;beat+=step)ticks.push(beat);
  const ruler=document.createElement('div');
  ruler.className='group-wave-ruler';
  ruler.setAttribute('aria-hidden','true');
  for(const beat of ticks){
    const tick=document.createElement('span');
    tick.style.left=((beat-start)/(end-start)*100)+'%';
    tick.textContent=beat===end?'':String(beat);
    ruler.append(tick);
  }
  const rulerRow=document.createElement('div');
  rulerRow.className='group-wave-row group-wave-header';
  const unit=document.createElement('span');
  unit.textContent='拍';
  rulerRow.append(unit,ruler);
  const rows=[];
  for(const {track,clips} of tracks.values()){
    const row=document.createElement('div'),label=document.createElement('strong'),canvas=document.createElement('canvas');
    row.className='group-wave-row';
    row.dataset.track=track.id;
    label.textContent=track.name;
    label.title=track.name;
    canvas.setAttribute('aria-label',track.name+'：'+clips.map(clip=>clip.name).join('、'));
    canvas.setAttribute('aria-description','左右方向键定位播放');
    canvas.tabIndex=0;
    canvas.title='点击定位播放';
    const seek=beat=>{
      arrangementSeek(Math.max(start,Math.min(end,beat)));
      updateGroupPlayhead();
    };
    canvas.addEventListener('click',event=>{
      const bounds=canvas.getBoundingClientRect();
      if(!bounds.width)return;
      seek(start+(event.clientX-bounds.left)/bounds.width*(end-start));
    });
    canvas.addEventListener('keydown',event=>{
      if(event.key!=='ArrowLeft'&&event.key!=='ArrowRight')return;
      event.preventDefault();
      event.stopPropagation();
      const beat=cursorBeat<start||cursorBeat>end?start:cursorBeat;
      seek(beat+(event.key==='ArrowRight'?1:-1)*Number($('quantize').value));
    });
    row.append(label,canvas);
    rows.push({track,clips,canvas,row});
  }
  const overlay=document.createElement('div'),indicator=document.createElement('i');
  overlay.className='group-wave-overlay';
  overlay.setAttribute('aria-hidden','true');
  overlay.append(indicator);
  groupList.replaceChildren(rulerRow,...rows.map(item=>item.row),overlay);
  groupTimeline={start,end,rows,indicator,ticks};
  queueAudioGroup();
}

audio.addEventListener('timeupdate',updateGroupPlayhead);
document.addEventListener('clip-waveform-ready',()=>{
  queueAudioGroup();
  drawSelectedWaveform();
});
groupList.parentElement.addEventListener('scroll',queueAudioGroup,{passive:true});

function drawSelectedWaveform(){
  const selected=selectedAudioClip(),canvas=$('selectedWaveform');
  if(!selected||canvas.hidden||scorePanel.hidden)return;
  const width=canvas.parentElement.clientWidth,height=canvas.parentElement.clientHeight;
  if(!width||!height)return;
  const ctx=canvasSize(canvas,width,height);
  ctx.fillStyle='#151517';ctx.fillRect(0,0,width,height);
  const centre=height/2;
  ctx.fillStyle='#303034';ctx.fillRect(0,centre,width,1);
  const waveform=clipWaveform(selected.track,selected.clip);
  if(!waveform)return;
  const {first,end,peaks,stride,begin}=waveform;
  ctx.strokeStyle=trackColor(selected.track);ctx.beginPath();
  for(let i=first;i<end;i++){
    const [lo,hi]=peaks[i];
    const fraction=Math.max(0,Math.min(1,((i+.5)*stride-begin)/selected.clip.frames));
    const x=fraction*width;
    ctx.moveTo(x,centre-hi*height*.43);ctx.lineTo(x,centre-lo*height*.43);
  }
  ctx.stroke();
}

function renderDetails(){
  const selected=selectedAudioClip(),group=clipGroupItems();
  const showAudio=selected||group.length||isAudioTrack(tr());
  scorePanel.dataset.details=showAudio?'audio':'notes';
  audioDetail.hidden=!showAudio;
  if(!showAudio){detailKey='';return}
  $('trackTitle').textContent=selected?selected.clip.name:group.length?`${group.length} 个片段`:tr().name;
  $('noteCount').textContent=selected?(selected.clip.frames/clipRate).toFixed(3)+' 秒':group.length?new Set(group.map(item=>item.track.id)).size+' 轨':'';
  $('selectedWaveform').hidden=!selected;
  $('inlineClipFields').hidden=!selected;
  $('inlineGroupFields').hidden=group.length<2;
  $('clipGroupList').hidden=!!selected;
  const key=JSON.stringify([project?.id,project?.revision,clipSelectionRefs()]);
  if(detailKey!==key){
    if(selected){
      $('inlineClipGain').value=selected.clip.gain_db;
      $('inlineClipIn').value=Number((selected.clip.fade_in/48).toFixed(3));
      $('inlineClipOut').value=Number((selected.clip.fade_out/48).toFixed(3));
    }else{
      renderAudioGroup(group);
      $('inlineGroupGain').value='0';
    }
    detailKey=key;
  }
  drawSelectedWaveform();
  queueAudioGroup();
}

$('inlineClipApply').onclick=()=>{
  if(!selectedAudioClip())return;
  const values=['inlineClipGain','inlineClipIn','inlineClipOut'].map(id=>$(id).valueAsNumber);
  if(values.some(value=>!Number.isFinite(value))||values.slice(1).some(value=>value<0)){toast('请输入有效的增益和淡化',true);return}
  const [gain_db,first,last]=values;
  void quickClipAction({...clipRequest(),action:{kind:'set',gain_db,fade_in:Math.round(first*48),fade_out:Math.round(last*48)}});
};
$('inlineGroupApply').onclick=()=>{
  const delta_db=$('inlineGroupGain').valueAsNumber;
  if(Number.isFinite(delta_db)&&delta_db!==0)void quickGroupAction(groupRequest({kind:'gain',delta_db}));
};
new ResizeObserver(drawSelectedWaveform).observe(audioDetail);
new ResizeObserver(queueAudioGroup).observe(groupList);
new ResizeObserver(()=>{if(project&&!scorePanel.hidden)draw()}).observe(scorePanel);
scorePanel.querySelector('.roll-column').addEventListener('pointerdown',()=>{workspaceFocus='editor';publishSelection()},true);
const beforeWorkspaceRefresh=refresh;
refresh=function(external=false){beforeWorkspaceRefresh(external);if(project)renderDetails()};
