'use strict';

const takeButton=document.createElement('button');
takeButton.id='groupTakes';
takeButton.hidden=true;
takeButton.innerHTML=icon('wave')+' 录音 A/B';
groupActions.prepend(takeButton);

const takeDialog=document.createElement('dialog');
takeDialog.id='takeDialog';
takeDialog.className='arrange-dialog';
takeDialog.innerHTML=`
  <div class="dialog-heading"><h2>录音 A/B</h2><button id="takeClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="clip-tools">
    <label>起拍<input id="takeStart" type="number" min="0" step="any"></label>
    <label>止拍<input id="takeEnd" type="number" min="0" step="any"></label>
    <label>电平<select id="takeLevel"><option value="original">原电平</option><option value="matched">匹配响度</option></select></label>
  </div>
  <div class="clip-tools take-buttons">
    <button id="takeA" aria-pressed="false"></button><button id="takeB" aria-pressed="false"></button>
    <button id="takeStop" aria-label="停止试听">${icon('stop')}</button>
    <button id="takeExport" disabled>导出 A/B</button>
  </div>
  <output id="takeStatus" role="status"></output>
  <div class="arrange-dialog-footer">
    <label>拼接轨 <select id="takeTrack"></select></label>
    <button id="takeUseA">收录 A</button><button id="takeUseB">收录 B</button>
  </div>`;
document.body.append(takeDialog);
wireDialog(takeDialog);
let takeReview=null;

const takeRange=()=>[$('takeStart').valueAsNumber,$('takeEnd').valueAsNumber];
const takeCurrent=review=>project?.id===review.binding.project_id&&project.revision===review.binding.expected_revision;

function paintTakeReview(){
  const active=!!takeReview?.pair&&audio.auditionId===takeReview.pair.id&&!audio.paused;
  $('takeA').setAttribute('aria-pressed',String(active&&audio.auditionVariant===0));
  $('takeB').setAttribute('aria-pressed',String(active&&audio.auditionVariant===1));
  $('takeStop').disabled=!active;
  $('takeExport').disabled=!takeReview?.pair||takeReview.busy;
}

function takeBusy(review,busy){
  review.busy=busy;
  if(takeReview!==review)return;
  $('takeExport').disabled=busy||!review.pair;
  for(const id of ['takeA','takeB','takeStart','takeEnd','takeLevel','takeTrack','takeUseA','takeUseB']){
    $(id).disabled=busy;
  }
}

async function releaseTakePair(review){
  const pair=review.pair;
  review.pair=null;
  if(takeReview===review)paintTakeReview();
  if(!pair)return;
  await audio.stopAudition(pair.id).catch(()=>{});
  await api('range_audition',{...review.binding,action:'release',audition_id:pair.id}).catch(()=>{});
}

takeButton.onclick=async()=>{
  if(clipBusy||takeDialog.open)return;
  await editChain.catch(()=>{});
  const items=clipGroupItems();
  if(items.length!==2||items.some(item=>item.clip.beat_length))return;
  const common=[Math.max(...items.map(item=>item.clip.start)),Math.min(...items.map(item=>clipEndBeat(item.clip)))];
  if(common[1]<=common[0]){toast('两遍录音需要覆盖同一拍段',true);return}
  const box=arrangeSelection;
  const chosen=box&&items.every(item=>box.ids.includes(item.track.id))
    ?[Math.max(common[0],box.start),Math.min(common[1],box.end)]:common;
  let [start,end]=chosen[0]<chosen[1]?chosen:common;
  end=Math.min(end,secondsBeat(beatSeconds(start)+45));
  const recordings=items.map(({track,clip})=>({track_id:track.id,clip_id:clip.id}));
  takeReview={binding:{project_id:project.id,expected_revision:project.revision},recordings,pair:null,busy:false};
  $('takeStart').value=start;$('takeEnd').value=end;
  $('takeA').textContent='A · '+items[0].clip.name;
  $('takeB').textContent='B · '+items[1].clip.name;
  $('takeTrack').replaceChildren(new Option('新建拼接轨',''),...project.tracks
    .filter(track=>isAudioTrack(track)&&!recordings.some(item=>item.track_id===track.id))
    .map(track=>new Option(track.name,track.id)));
  $('takeStatus').textContent='';
  takeBusy(takeReview,false);
  paintTakeReview();
  takeDialog.showModal();
};

async function playTake(index){
  const review=takeReview;
  if(!review||review.busy)return;
  takeBusy(review,true);
  try{
    if(!takeCurrent(review))throw new Error('工程已更新，请重新选择录音');
    const beat_range=takeRange();
    const level_match=$('takeLevel').value==='matched';
    const signature=JSON.stringify([beat_range,level_match]);
    if(review.signature!==signature||!review.pair){
      $('takeStatus').textContent='准备同段试听…';
      await releaseTakePair(review);
      review.pair=await api('range_audition',{...review.binding,action:'prepare',
        recordings:review.recordings,beat_ranges:[beat_range,beat_range],level_match});
      if(takeReview!==review||!takeCurrent(review)){await releaseTakePair(review);return}
      review.signature=signature;
    }
    await audio.auditionSide(review.pair.id,index===0?'A':'B');
    if(takeReview===review)$('takeStatus').textContent='';
  }catch(error){if(takeReview===review)$('takeStatus').textContent=error.message}
  finally{takeBusy(review,false);paintTakeReview()}
}

async function useTake(index){
  const review=takeReview;
  if(!review||review.busy)return;
  takeBusy(review,true);
  try{
    if(!takeCurrent(review))throw new Error('工程已更新，请重新选择录音');
    const to_track_id=$('takeTrack').value;
    const request={...review.binding,clips:[review.recordings[index]],action:{
      kind:'extract',beat_range:takeRange(),...(to_track_id?{to_track_id}:{})
    }};
    const preview=await api('audio_clip_group',request);
    if(takeReview!==review)return;
    await releaseTakePair(review);
    if(takeReview!==review)return;
    await applyClipGroup(request,preview.preview_id);
    if(takeReview===review)takeDialog.close();
  }catch(error){if(takeReview===review)$('takeStatus').textContent=clipError(error.message)}
  finally{takeBusy(review,false)}
}

$('takeA').onclick=()=>void playTake(0);
$('takeB').onclick=()=>void playTake(1);
$('takeUseA').onclick=()=>void useTake(0);
$('takeUseB').onclick=()=>void useTake(1);
$('takeStop').onclick=()=>{if(takeReview?.pair)void audio.stopAudition(takeReview.pair.id)};
$('takeExport').onclick=async()=>{
  const review=takeReview;
  if(!review?.pair||review.busy||!takeCurrent(review))return;
  takeBusy(review,true);
  try{
    const result=await api('range_audition',{...review.binding,action:'export',audition_id:review.pair.id});
    if(takeReview===review)await download(result.url);
  }catch(error){if(takeReview===review)$('takeStatus').textContent=error.message}
  finally{takeBusy(review,false)}
};
$('takeClose').onclick=()=>takeDialog.close();
for(const id of ['takeStart','takeEnd','takeLevel'])$(id).onchange=()=>{
  if(takeReview&&!takeReview.busy)void releaseTakePair(takeReview);
};
takeDialog.addEventListener('close',()=>{
  const previous=takeReview;
  takeReview=null;
  if(previous)void releaseTakePair(previous);
});
audio.addEventListener('timeupdate',()=>{if(takeDialog.open)paintTakeReview()});
