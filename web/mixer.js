'use strict';

const mixerButton=document.createElement('button');
mixerButton.id='showMixer';
mixerButton.title='多轨混音 · F5';
mixerButton.setAttribute('aria-pressed','false');
mixerButton.innerHTML=icon('sliders')+'<span>混音</span>';
$('showEffects').after(mixerButton);

const mixerPanel=document.createElement('section');
mixerPanel.className='studio-mixer';
mixerPanel.hidden=true;
mixerPanel.setAttribute('aria-label','多轨混音器');
mixerPanel.innerHTML=`<section class="mixer-channel mixer-master" aria-label="总输出">
    <label class="mixer-master-title" for="masterGain">总输出</label>
    <div class="mixer-fader"><input id="masterGain" type="range" class="mixer-gain" min="-60" max="12" step="0.1"></div>
    <output id="masterGainValue" for="masterGain"></output>
    <button id="resetMasterGain" type="button" aria-label="总输出恢复到 0 dB">重置</button>
  </section><div id="mixerStrips"></div>`;
scorePanel.append(mixerPanel);

const masterGain=$('masterGain'),masterGainValue=$('masterGainValue');
const channelViews=new Map();
let mixerProject=null,mixerGesture=null,mixerTimer,mixerViewEpoch=0;
const dbText=value=>(value>0?'+':'')+value.toFixed(1)+' dB';
const panText=value=>value===0?'居中':(value<0?'L ':'R ')+Math.round(Math.abs(value)*100);

function restoreMixerDraft(binding){
  if(audio.liveMix&&project?.id===binding.project_id&&project.revision===binding.expected_revision){
    void audio.monitorMix(project,rendered?.job_id,{tracks:[]}).catch(error=>toast(error.message,true));
  }
}

function cancelMixerGesture(){
  clearTimeout(mixerTimer);
  mixerTimer=null;
  const draft=mixerGesture;
  mixerGesture=null;
  if(draft){
    restoreMixerDraft(draft);
    refreshMixer();
  }
}

function previewMixerValue(id,field,input,output,format){
  if(!project||id!==null&&!project.tracks.some(item=>item.id===id))return;
  if(mixerGesture&&(mixerGesture.track_id!==id||mixerGesture.field!==field))cancelMixerGesture();
  const value=input.valueAsNumber;
  if(!Number.isFinite(value))return;
  if(!mixerGesture)mixerGesture={track_id:id,field,project_id:project.id,expected_revision:project.revision};
  mixerGesture.value=value;
  output.textContent=format(value);
  if(!window.desktopNative||!audio.liveMix||audio.paused||mixerTimer)return;
  mixerTimer=setTimeout(()=>{
    mixerTimer=null;
    const draft=mixerGesture;
    if(!draft||project?.id!==draft.project_id||project.revision!==draft.expected_revision)return;
    const parameters=draft.track_id===null
      ?{tracks:[],master_gain_db:draft.value}
      :{tracks:[{track_id:draft.track_id,[draft.field]:draft.value}]};
    void audio.monitorMix(project,rendered?.job_id,parameters)
      .catch(error=>{if(mixerGesture===draft)toast(error.message,true)});
  },32);
}

async function commitMixerValue(id,field){
  const draft=mixerGesture;
  if(!draft||draft.track_id!==id||draft.field!==field)return;
  clearTimeout(mixerTimer);mixerTimer=null;
  mixerGesture=null;
  const previous=id===null?project?.master_gain_db??0:project?.tracks.find(item=>item.id===id)?.[field];
  if(project?.id===draft.project_id&&project.revision===draft.expected_revision&&previous===draft.value){
    restoreMixerDraft(draft);
    refreshMixer();
    return;
  }
  try{
    const values={[field]:draft.value};
    await edit([id===null?{op:'set_project',values}:{op:'update_track',track_id:id,values}],draft);
  }catch{
    restoreMixerDraft(draft);
    refreshMixer();
  }
}

function wireMixerFader(id,field,input,output,format){
  input.oninput=()=>previewMixerValue(id,field,input,output,format);
  input.onchange=()=>void commitMixerValue(id,field);
  input.onpointercancel=cancelMixerGesture;
  input.onkeydown=event=>{
    if(event.key==='Escape'&&mixerGesture){event.preventDefault();cancelMixerGesture()}
  };
}

wireMixerFader(null,'master_gain_db',masterGain,masterGainValue,dbText);
$('resetMasterGain').onclick=()=>{
  masterGain.value=0;
  masterGain.oninput();
  masterGain.onchange();
};

function makeChannel(id){
  const strip=document.createElement('section');
  strip.className='mixer-channel';
  strip.dataset.track=id;
  strip.innerHTML=`<button class="mixer-name"><span class="mixer-track-name"></span><span class="mixer-selection" aria-hidden="true">编辑中</span></button>
    <div class="mixer-controls"><button data-switch="mute">M</button><button data-switch="solo">S</button></div>
    <div class="mixer-fader"><input type="range" class="mixer-gain" min="-60" max="12" step="0.1"><div class="mixer-meter" aria-hidden="true"><i></i><i></i></div></div>
    <output class="mixer-db"></output><input type="range" class="mixer-pan" min="-1" max="1" step="0.01"><output class="mixer-balance"></output>`;
  const name=strip.querySelector('.mixer-name'),title=name.querySelector('.mixer-track-name');
  const controls=[...strip.querySelectorAll('[data-switch]')];
  const gain=strip.querySelector('.mixer-gain'),db=strip.querySelector('.mixer-db');
  const pan=strip.querySelector('.mixer-pan'),balance=strip.querySelector('.mixer-balance');
  const meter=[...strip.querySelectorAll('.mixer-meter i')];
  name.onclick=()=>[...$('tracks').querySelectorAll('.track-card')].find(el=>el.dataset.track===id)?.click();
  for(const button of controls)button.onclick=()=>{
    const track=project?.tracks.find(item=>item.id===id);
    if(!track)return;
    const field=button.dataset.switch;
    void edit([{op:'update_track',track_id:id,values:{[field]:!track[field]}}]).catch(()=>{});
  };
  for(const [input,output,field,format] of [[gain,db,'gain_db',dbText],[pan,balance,'pan',panText]])
    wireMixerFader(id,field,input,output,format);
  strip.addEventListener('click',event=>{
    if(!event.target.closest('button,input'))name.click();
  });
  return {strip,name,title,controls,gain,pan,db,balance,meter};
}

function refreshMixer(){
  if(!project||mixerPanel.hidden)return;
  if(mixerProject!==project.id){
    mixerProject=project.id;
    mixerGesture=null;
    channelViews.clear();
    $('mixerStrips').replaceChildren();
  }
  if(mixerGesture&&mixerGesture.expected_revision!==project.revision)mixerGesture=null;
  if(mixerGesture?.track_id!==null){
    const saved=project.master_gain_db??0;
    masterGain.value=saved;
    masterGainValue.textContent=dbText(saved);
  }
  const ids=new Set(project.tracks.map(t=>t.id));
  for(const [id,view] of channelViews)if(!ids.has(id)){view.strip.remove();channelViews.delete(id)}
  const strips=$('mixerStrips');
  let nextStrip=strips.firstElementChild;
  for(const track of project.tracks){
    let view=channelViews.get(track.id);
    if(!view){
      view=makeChannel(track.id);
      channelViews.set(track.id,view);
    }
    if(view.strip!==nextStrip)strips.insertBefore(view.strip,nextStrip);
    nextStrip=view.strip.nextElementSibling;
    view.title.textContent=track.name;
    view.name.title=track.name;
    const active=track.id===trackId;
    view.name.setAttribute('aria-pressed',String(active));
    view.strip.style.setProperty('--strip-color',trackColor(track));
    view.strip.classList.toggle('selected',active);
    for(const button of view.controls){
      const mute=button.dataset.switch==='mute';
      button.setAttribute('aria-label',(mute?'静音 ':'独奏 ')+track.name);
      button.setAttribute('aria-pressed',String(track[button.dataset.switch]));
    }
    view.gain.setAttribute('aria-label','电平 '+track.name);
    view.pan.setAttribute('aria-label','声像 '+track.name);
    if(mixerGesture?.track_id!==track.id||mixerGesture.field!=='gain_db'){
      view.gain.value=track.gain_db;
      view.db.textContent=dbText(track.gain_db);
    }
    if(mixerGesture?.track_id!==track.id||mixerGesture.field!=='pan'){
      view.pan.value=track.pan;
      view.balance.textContent=panText(track.pan);
    }
  }
  showMixBuses();
}

function paintMixerMeters(){
  if(mixerPanel.hidden||scorePanel.hidden)return;
  const levels=new Map(audio.monitorTrackIds?.map((id,index)=>[id,audio.trackMeters[index]])||[]);
  for(const [id,view] of [...channelViews,...returnViews]){
    const peak=levels.get(id)||[];
    view.meter.forEach((bar,index)=>{
      const db=20*Math.log10(Math.max(peak[index]||0,.001));
      bar.style.transform=`scaleY(${Math.max(0,Math.min(1,1+db/60))})`;
    });
  }
}

function mixerFocusTarget(id){
  return mixerPanel.hidden||scorePanel.hidden?null:(channelViews.get(id)??returnViews.get(id))?.name;
}

mixerButton.onclick=async()=>{
  const opening=mixerButton.getAttribute('aria-pressed')!=='true'||scorePanel.hidden;
  const epoch=++mixerViewEpoch;
  mixerButton.setAttribute('aria-pressed',String(opening));
  routingButton.hidden=!opening;
  mixerPanel.getAnimations().forEach(animation=>animation.cancel());
  if(opening){
    mixerPanel.hidden=false;
    scorePanel.dataset.mixer='true';
    await setDetailsVisible(true);
    if(epoch!==mixerViewEpoch)return;
    refreshMixer();paintMixerMeters();
    animateElement(mixerPanel,[{opacity:0,transform:'translateY(6px)'},{opacity:1,transform:'translateY(0)'}],'panelIn');
  }else{
    cancelMixerGesture();
    await animateElement(mixerPanel,[{opacity:1},{opacity:0,transform:'translateY(6px)'}],'panelOut').finished.catch(()=>{});
    if(epoch!==mixerViewEpoch)return;
    mixerPanel.hidden=true;
    scorePanel.dataset.mixer='false';
    draw();
  }
};
const renderBeforeMixer=refresh;
refresh=function(external=false){renderBeforeMixer(external);refreshMixer()};
audio.addEventListener('timeupdate',paintMixerMeters);
audio.addEventListener('pause',paintMixerMeters);
audio.addEventListener('ended',paintMixerMeters);
