'use strict';

const warpDialog=document.createElement('dialog');
warpDialog.id='warpDialog';
warpDialog.className='arrange-dialog';
warpDialog.innerHTML=`
  <div class="dialog-heading"><h2>片段对拍</h2><button id="closeWarp" aria-label="关闭">${icon('close')}</button></div>
  <div class="clip-tools">
    <label>片段拍数<input id="warpBeats" type="number" min="0" step="any"></label>
    <button id="addWarpPoint" title="也可双击源波形添加">添加点</button>
    <select id="warpAttacks" aria-label="按起音添加对拍点" hidden></select>
  </div>
  <svg id="warpPlot" viewBox="0 0 432 130" role="img" aria-label="源波形与目标拍位">
    <path id="warpPeaks"/>
    <path d="M12 46H420 M12 110H420" class="warp-axis"/>
    <text x="12" y="17">源波形</text><text x="12" y="102">目标拍位</text>
    <g id="warpAttackMarkers" aria-hidden="true"></g>
    <g id="warpLines"></g>
  </svg>
  <div id="warpRows"></div>
  <output id="warpStatus" role="status"></output>
  <div class="arrange-dialog-footer"><button id="previewWarp">预览</button><button id="applyWarp" class="primary" disabled>应用</button></div>`;
document.body.append(warpDialog);
wireDialog(warpDialog);
const warpAudition=arrangementAuditionControls(warpDialog);
const warpSvg='http://www.w3.org/2000/svg';
let warpTarget=null,warpDraft=null,warpPoints=[],warpDrag=null;

function showWarpAttacks(attacks,limited=false){
  const select=$('warpAttacks'),markers=$('warpAttackMarkers');
  select.replaceChildren(new Option(limited?'起音（部分）':'起音',''));
  select.hidden=!attacks.length;
  markers.replaceChildren(...attacks.map(({frame,beat})=>{
    const label=(frame/48).toFixed(1)+' ms · '+Number(beat.toFixed(3))+' 拍';
    select.add(new Option(label,String(frame)));
    const x=12+408*frame/warpTarget.clip.frames;
    const mark=document.createElementNS(warpSvg,'path');
    const title=document.createElementNS(warpSvg,'title');
    title.textContent=label;
    mark.append(title);
    mark.setAttribute('d',`M${x} 20V35`);
    mark.dataset.attackFrame=frame;
    return mark;
  }));
}

function plotWarp(){
  const beats=$('warpBeats').valueAsNumber,frames=warpTarget?.clip.frames||1;
  const group=$('warpLines');
  group.replaceChildren(...warpPoints.flatMap(({frame,beat},index)=>{
    if(!(frame>=0&&frame<=frames&&beat>=0&&beat<=beats))return [];
    const sourceX=12+408*frame/frames,targetX=12+408*beat/beats;
    const line=document.createElementNS(warpSvg,'path'),knob=document.createElementNS(warpSvg,'circle');
    line.setAttribute('d',`M${sourceX} 23V72L${targetX} 110`);
    knob.setAttribute('cx',targetX);
    knob.setAttribute('cy',110);
    knob.setAttribute('r',5);
    knob.dataset.warpIndex=index;
    return [line,knob];
  }));
}
function warpState(message=''){
  $('warpStatus').textContent=message;
  $('warpBeats').disabled=$('addWarpPoint').disabled=$('previewWarp').disabled=clipBusy;
  $('warpAttacks').disabled=clipBusy;
  $('applyWarp').disabled=clipBusy||!warpDraft;
  for(const control of $('warpRows').querySelectorAll('input,button'))control.disabled=clipBusy;
}
function invalidateWarp(){
  warpDraft=null;
  warpAudition.set(null);
  plotWarp();
  warpState();
}
function warpRows(){
  $('warpRows').replaceChildren(...warpPoints.map((point,index)=>{
    const row=document.createElement('div');
    row.innerHTML=`<label>源位置 · ms<input type="number" min="0" step="any"></label>
      <label>相对拍位<input type="number" min="0" step="any"></label>
      <button aria-label="移除对拍点" title="移除">${icon('close')}</button>`;
    const [source,destination]=row.querySelectorAll('input');
    source.value=Number((point.frame/48).toFixed(5));
    destination.value=point.beat;
    source.oninput=()=>{point.frame=Math.round(source.valueAsNumber*48);invalidateWarp()};
    destination.oninput=()=>{point.beat=destination.valueAsNumber;invalidateWarp()};
    row.querySelector('button').onclick=()=>{warpPoints.splice(index,1);warpRows();invalidateWarp()};
    return row;
  }));
}
function placeWarpPoint(frame,currentBeat=null){
  if(!warpTarget||clipBusy||!(frame>0&&frame<warpTarget.clip.frames)||warpPoints.some(point=>point.frame===frame))return;
  const draft={...warpTarget.clip,beat_length:$('warpBeats').valueAsNumber,warp_points:warpPoints};
  if(!(draft.beat_length>0))return;
  const beat=currentBeat!==null&&!warpPoints.length
    && draft.beat_length===warpTarget.originalBeats
    ? currentBeat-warpTarget.clip.start
    : mapClipWarp(draft,'frame','beat',frame);
  warpPoints.push({frame,beat:Number(beat.toFixed(7))});
  warpPoints.sort((a,b)=>a.frame-b.frame);
  warpRows();
  invalidateWarp();
}

$('clipWarp').onclick=async()=>{
  if(clipBusy)return;
  await editChain.catch(()=>{});
  const selected=selectedAudioClip();
  if(!selected)return;
  const target={request:groupRequest({kind:'warp'}),clip:selected.clip,attacks:[],
    originalBeats:clipEndBeat(selected.clip)-selected.clip.start};
  warpTarget=target;
  warpPoints=structuredClone(target.clip.warp_points||[]);
  $('warpBeats').value=target.originalBeats;
  showWarpAttacks([]);
  $('warpPeaks').setAttribute('d','');
  warpRows();
  invalidateWarp();
  warpDialog.showModal();
  try{
    const wave=await api('audio_waveform',{project_id:target.request.project_id,
      expected_revision:target.request.expected_revision,track_id:selected.track.id,
      clip_id:target.clip.id,include_transients:true});
    if(warpTarget!==target)return;
    target.attacks=wave.transients;
    showWarpAttacks(target.attacks,wave.transients_limited);
    $('warpPeaks').setAttribute('d',wave.peaks.map(([lo,hi],i)=>{
      const x=(12+408*i/Math.max(1,wave.peaks.length-1)).toFixed(2);
      return 'M'+x+' '+(46-26*hi).toFixed(2)+'V'+(46-26*lo).toFixed(2);
    }).join(''));
  }catch(error){if(warpTarget===target)warpState(error.message)}
};
$('closeWarp').onclick=()=>warpDialog.close();
warpDialog.addEventListener('close',()=>{warpTarget=null;warpDraft=null;warpDrag=null});
$('warpBeats').oninput=invalidateWarp;
$('addWarpPoint').onclick=()=>{
  if(!warpTarget)return;
  const frames=[0,...warpPoints.map(point=>point.frame).filter(Number.isFinite).sort((a,b)=>a-b),warpTarget.clip.frames];
  let best=0;
  for(let i=1;i<frames.length-1;i++)if(frames[i+1]-frames[i]>frames[best+1]-frames[best])best=i;
  placeWarpPoint(Math.round((frames[best]+frames[best+1])/2));
};
const warpPlot=$('warpPlot');
function placeWarpAttack(frame){
  const attack=warpTarget?.attacks.find(item=>item.frame===frame);
  if(attack)placeWarpPoint(frame,attack.beat);
}
$('warpAttacks').onchange=event=>{
  const select=event.currentTarget;
  if(select.value)placeWarpAttack(Number(select.value));
  select.value='';
};
$('warpAttackMarkers').onclick=event=>{
  const frame=event.target.dataset.attackFrame;
  if(frame!==undefined)placeWarpAttack(Number(frame));
};
function warpPlotPosition(event){
  const transform=warpPlot.getScreenCTM().inverse();
  const point=new DOMPoint(event.clientX,event.clientY).matrixTransform(transform);
  return {fraction:Math.max(0,Math.min(1,(point.x-12)/408)),y:point.y};
}
warpPlot.ondblclick=event=>{
  if(!warpTarget||event.target.dataset.attackFrame!==undefined)return;
  const point=warpPlotPosition(event);
  if(point.y<78)placeWarpPoint(Math.round(point.fraction*warpTarget.clip.frames));
};
warpPlot.onpointerdown=event=>{
  const index=event.target.dataset.warpIndex;
  if(clipBusy||index===undefined)return;
  warpDrag=Number(index);
  warpPlot.setPointerCapture(event.pointerId);
};
warpPlot.onpointermove=event=>{
  if(warpDrag===null||clipBusy)return;
  const beats=$('warpBeats').valueAsNumber;
  const min=(warpPoints[warpDrag-1]?.beat||0)+.00001,max=(warpPoints[warpDrag+1]?.beat||beats)-.00001;
  if(!(max>=min))return;
  const value=Math.round(warpPlotPosition(event).fraction*beats*96)/96;
  warpPoints[warpDrag].beat=Number(Math.max(min,Math.min(max,value)).toFixed(7));
  $('warpRows').children[warpDrag].querySelectorAll('input')[1].value=warpPoints[warpDrag].beat;
  invalidateWarp();
};
warpPlot.onpointerup=warpPlot.onpointercancel=()=>{warpDrag=null};

$('previewWarp').onclick=async()=>{
  const target=warpTarget,beats=$('warpBeats').valueAsNumber;
  if(!target||clipBusy)return;
  warpDraft=null;
  warpAudition.set(null);
  if(!(beats>0)||!warpPoints.every(point=>Number.isFinite(point.frame)&&Number.isFinite(point.beat))){
    warpState('请填写拍数和各个对拍点');
    return;
  }
  clipBusy=true;
  warpState('正在核对对拍点…');
  let message;
  try{
    const request={...target.request,action:{kind:'warp',beats,points:structuredClone(warpPoints)}};
    const preview=await api('audio_clip_group',request);
    if(warpTarget!==target)return;
    warpDraft={request,id:preview.preview_id};
    bindClipAudition(warpAudition,request,preview.preview_id);
    message=preview.warp.anchors.length+' 个对拍点 · '+beats+' 拍';
  }catch(error){message=clipError(error.message)}
  finally{clipBusy=false;warpState(warpTarget===target?message:'')}
};
$('applyWarp').onclick=async()=>{
  const draft=warpDraft,target=warpTarget;
  if(clipBusy||!draft||!target)return;
  clipBusy=true;
  warpState('正在保存…');
  let message;
  try{
    await applyClipGroup(draft.request,draft.id,warpAudition.binding());
    if(warpTarget===target)warpDialog.close();
  }catch(error){warpDraft=null;message=clipError(error.message)}
  finally{clipBusy=false;warpState(warpTarget===target?message:'')}
};
