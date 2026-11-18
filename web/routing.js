'use strict';

const routingButton=document.createElement('button');
routingButton.id='showRouting';
routingButton.hidden=true;
routingButton.innerHTML=icon('sliders')+'<span>路由</span>';
routingButton.title='混音编组与发送';
mixerButton.after(routingButton);

const routingDialog=document.createElement('dialog');
routingDialog.className='studio-routing';
routingDialog.setAttribute('aria-labelledby','routingTitle');
routingDialog.innerHTML='<div class="dialog-heading"><h2 id="routingTitle">混音路由</h2><button id="closeRouting" aria-label="关闭">'+icon('close')+'</button></div>'
  +'<div class="routing-toolbar"><select id="routingBus" aria-label="编组或返回通道"></select><button id="routingGroup">＋ 编组</button><button id="routingRoom">＋ 混响</button><button id="routingDelay">＋ 延迟</button></div>'
  +'<div id="routingBody"></div><div class="routing-footer"><output id="routingMessage" role="status"></output><button id="routingApply" class="primary">应用</button></div>';
document.body.append(routingDialog);
wireDialog(routingDialog);

let routingDraft,routingSpecs,routingTimer,routingSaving=false,routingOpening=false;
const returnViews=new Map();

function routingChanges(){
  const draft=routingDraft;
  const settings={tracks:[]};
  if(!draft)return settings;
  if(JSON.stringify(draft.buses)!==draft.originalBuses)settings.aux_buses=draft.buses;
  if(JSON.stringify(draft.groups)!==draft.originalGroups)settings.mix_groups=draft.groups;
  for(const track of draft.tracks){
    const change={track_id:track.id};
    const sends=draft.sends.get(track.id),group=draft.outputs.get(track.id);
    if(JSON.stringify(sends)!==JSON.stringify(track.sends||[]))change.sends=sends;
    if(group!==(track.output_group??null))change.output_group=group;
    if('sends' in change||'output_group' in change)settings.tracks.push(change);
  }
  return settings;
}

function routingTopology(draft){
  return [draft.groups.map(bus=>bus.id),draft.buses.map(bus=>bus.id)].map(ids=>ids.join(',')).join('/');
}

function routingInputValid(){
  return [...routingDialog.querySelectorAll('input:not(:disabled)')].every(input=>
    input.checkValidity()&&(input.type!=='text'||input.value.trim()!==''));
}

function routingChanged(){
  if(!routingDraft)return;
  const settings=routingChanges();
  const changed=!!(settings.aux_buses||settings.mix_groups||settings.tracks.length);
  const valid=routingInputValid();
  $('routingApply').disabled=routingSaving||!valid||!changed;
  clearTimeout(routingTimer);
  if(!valid||(routingSaving||!routingDialog.open)||audio.paused||!audio.liveMix)return;
  const draft=routingDraft;
  if(routingTopology(draft)!==draft.originalTopology){
    $('routingMessage').textContent='应用后重新播放';
    if(draft.previewRequested){restoreMixerDraft(draft);draft.previewRequested=false}
    return;
  }
  $('routingMessage').textContent='';
  if(!changed&&!draft.previewRequested)return;
  routingTimer=setTimeout(()=>{
    if(draft!==routingDraft||!routingDialog.open||project?.id!==draft.project_id||project.revision!==draft.expected_revision)return;
    draft.previewRequested=true;
    void audio.monitorMix(project,rendered?.job_id,settings).catch(error=>{
      if(draft===routingDraft&&routingDialog.open)$('routingMessage').textContent=error.message;
    });
  },60);
}

function routingNumber(value,min,max,label,onValue){
  const input=document.createElement('input');
  input.type='number';input.value=value;input.min=min;input.max=max;input.step='any';input.required=true;
  input.setAttribute('aria-label',label);
  input.oninput=()=>{
    if(input.checkValidity())onValue(input.valueAsNumber);
    routingChanged();
  };
  return input;
}

function renderRouting(preferredId=$('routingBus').value){
  const draft=routingDraft;
  const picker=$('routingBus'),body=$('routingBody');
  const all=[...draft.groups,...draft.buses];
  const isGroup=bus=>draft.groups.includes(bus);
  const optionLabel=bus=>bus.name+(isGroup(bus)?' · 编组':' · 返回');
  picker.replaceChildren(...all.map(bus=>new Option(optionLabel(bus),bus.id)));
  const bus=all.find(item=>item.id===preferredId)||all[0];
  picker.disabled=!bus;
  const atLimit=draft.buses.length>=routingSpecs.aux_routing.max_buses;
  $('routingRoom').disabled=atLimit;
  $('routingDelay').disabled=atLimit;
  $('routingGroup').disabled=draft.groups.length>=routingSpecs.mix_groups.max_groups;
  body.replaceChildren();
  if(!bus){
    body.textContent='添加编组或效果返回';
    routingChanged();
    return;
  }
  picker.value=bus.id;
  const grouped=isGroup(bus),labelType=grouped?'编组':'返回';
  const header=document.createElement('div');header.className='routing-return';
  const name=document.createElement('input');
  name.type='text';name.value=bus.name;name.maxLength=100;name.required=true;
  name.setAttribute('aria-label',labelType+'名称');
  name.oninput=()=>{
    if(name.value.trim()){
      bus.name=name.value.trim();
      picker.selectedOptions[0].textContent=optionLabel(bus);
    }
    routingChanged();
  };
  const [returnMin,returnMax]=grouped?routingSpecs.mix_groups.gain_db:routingSpecs.aux_routing.return_gain_db;
  const gain=routingNumber(bus.gain_db,returnMin,returnMax,labelType+'电平 dB',value=>{bus.gain_db=value});
  gain.title=labelType+'电平 · dB';
  const mute=document.createElement('button');
  mute.textContent='M';mute.title=labelType+'静音';mute.setAttribute('aria-label',mute.title);
  mute.setAttribute('aria-pressed',String(bus.mute));
  mute.onclick=()=>{bus.mute=!bus.mute;mute.setAttribute('aria-pressed',String(bus.mute));routingChanged()};
  const remove=fxCommand('close','移除'+labelType,()=>{
    if(grouped){
      draft.groups=draft.groups.filter(item=>item.id!==bus.id);
      for(const [id,target] of draft.outputs)if(target===bus.id)draft.outputs.set(id,null);
    }else{
      draft.buses=draft.buses.filter(item=>item.id!==bus.id);
      for(const [id,sends] of draft.sends)draft.sends.set(id,sends.filter(send=>send.bus_id!==bus.id));
    }
    renderRouting();
  });
  header.append(name,gain,mute,remove);
  const list=document.createElement('div');list.className='routing-list';
  const [sendMin,sendMax]=routingSpecs.aux_routing.send_gain_db;
  for(const track of draft.tracks){
    const row=document.createElement('div');row.className='routing-row';row.dataset.track=track.id;
    const toggle=document.createElement('input');toggle.type='checkbox';
    toggle.id='routingSend_'+track.id;
    toggle.setAttribute('aria-label',(grouped?'编组 ':'发送 ')+track.name);
    const label=document.createElement('label');label.htmlFor=toggle.id;label.textContent=track.name;label.title=track.name;
    if(grouped){
      const target=draft.outputs.get(track.id);
      const route=document.createElement('span');
      const other=draft.groups.find(item=>item.id===target);
      toggle.checked=target===bus.id;
      route.className='routing-destination';
      route.textContent=toggle.checked?'':other?.name||'总线';
      route.title=route.textContent;
      toggle.onchange=()=>{
        draft.outputs.set(track.id,toggle.checked?bus.id:null);
        route.textContent=toggle.checked?'':'总线';
        route.title=route.textContent;
        routingChanged();
      };
      row.append(toggle,label,route);list.append(row);
      continue;
    }
    const sends=draft.sends.get(track.id);
    const current=sends.find(send=>send.bus_id===bus.id);
    toggle.checked=!!current;
    const value=routingNumber(current?.gain_db??-18,sendMin,sendMax,track.name+' 发送 dB',db=>{
      const send=sends.find(item=>item.bus_id===bus.id);
      if(send)send.gain_db=db;
    });
    value.disabled=!current;
    toggle.onchange=()=>{
      value.disabled=!toggle.checked;
      if(toggle.checked){
        if(!value.checkValidity())value.value=-18;
        sends.push({bus_id:bus.id,gain_db:value.valueAsNumber});
      }else{
        sends.splice(sends.findIndex(send=>send.bus_id===bus.id),1);
      }
      routingChanged();
    };
    row.append(toggle,label,value);list.append(row);
  }
  body.append(header,list);
  routingChanged();
}

async function openRouting(){
  if(!project||routingDialog.open||routingOpening)return;
  routingOpening=true;
  try{
    routingSpecs??=await api('effect_catalog');
    const buses=structuredClone(project.aux_buses||[]);
    const groups=structuredClone(project.mix_groups||[]);
    const sends=new Map(project.tracks.map(track=>[track.id,structuredClone(track.sends||[])]));
    routingDraft={project_id:project.id,expected_revision:project.revision,buses,groups,sends,tracks:project.tracks,
      outputs:new Map(project.tracks.map(track=>[track.id,track.output_group??null])),
      originalBuses:JSON.stringify(buses),originalGroups:JSON.stringify(groups)};
    routingDraft.originalTopology=routingTopology(routingDraft);
    $('routingMessage').textContent='';
    renderRouting();
    routingDialog.showModal();
  }catch(error){toast(error.message,true)}
  finally{routingOpening=false}
}

function addMixBus(kind,name){
  if(!routingDraft)return;
  const grouped=kind==='group';
  const buses=grouped?routingDraft.groups:routingDraft.buses;
  const limit=grouped?routingSpecs.mix_groups.max_groups:routingSpecs.aux_routing.max_buses;
  if(buses.length>=limit)return;
  const effect=routingSpecs.effects.find(item=>item.kind===kind);
  const bytes=crypto.getRandomValues(new Uint8Array(16));
  const id=(grouped?'group_':'aux_')+Array.from(bytes,value=>value.toString(16).padStart(2,'0')).join('');
  const names=new Set(buses.map(bus=>bus.name));
  let title=name;
  for(let index=2;names.has(title);index++)title=name+' '+index;
  buses.push({id,name:title,gain_db:0,mute:false,effects:effect?[{...effect.defaults,mix:1}]:[]});
  renderRouting(id);
}

async function applyRouting(){
  if(routingSaving||!routingDraft||!routingInputValid())return;
  const draft=routingDraft;
  const {tracks,...values}=structuredClone(routingChanges());
  const operations=tracks.map(({track_id,...values})=>({op:'update_track',track_id,values}));
  if(Object.keys(values).length)operations.unshift({op:'set_project',values});
  if(!operations.length)return;
  routingSaving=true;clearTimeout(routingTimer);
  $('routingApply').disabled=true;
  try{
    await edit(operations,draft);
    draft.applied=true;
    routingDialog.close();
  }catch(error){$('routingMessage').textContent=error.message}
  finally{routingSaving=false;routingChanged()}
}

const closeRouting=routingDialog.close.bind(routingDialog);
routingDialog.close=()=>{
  if(routingSaving&&!routingDraft?.applied)return;
  clearTimeout(routingTimer);
  if(routingDraft?.previewRequested&&!routingDraft.applied)restoreMixerDraft(routingDraft);
  closeRouting();
};
routingButton.onclick=()=>void openRouting();
$('closeRouting').onclick=()=>routingDialog.close();
$('routingBus').onchange=()=>renderRouting();
$('routingGroup').onclick=()=>addMixBus('group','编组');
$('routingRoom').onclick=()=>addMixBus('reverb','混响');
$('routingDelay').onclick=()=>addMixBus('delay','延迟');
$('routingApply').onclick=()=>void applyRouting();

function showMixBuses(){
  const groups=project.mix_groups||[];
  const buses=[...groups,...(project.aux_buses||[])];
  const ids=new Set(buses.map(bus=>bus.id));
  for(const [id,view] of returnViews)if(!ids.has(id)){view.strip.remove();returnViews.delete(id)}
  for(const bus of buses){
    let view=returnViews.get(bus.id);
    if(!view){
      const strip=document.createElement('section');
      strip.className='mixer-channel mixer-return';
      strip.dataset.bus=bus.id;
      strip.innerHTML='<button class="mixer-name"></button><div class="mixer-fader"><div class="mixer-meter" aria-hidden="true"><i></i><i></i></div></div><output></output>';
      const name=strip.querySelector('.mixer-name');
      name.onclick=()=>void openEffects(bus.id);
      const meter=[...strip.querySelectorAll('i')];
      view={strip,name,meter,level:strip.querySelector('output')};
      returnViews.set(bus.id,view);
    }
    const strips=$('mixerStrips');
    const previous=buses[buses.indexOf(bus)-1];
    const at=previous?returnViews.get(previous.id).strip.nextSibling:strips.querySelector('.mixer-return');
    if(at!==view.strip)strips.insertBefore(view.strip,at);
    view.name.textContent=bus.name;
    view.name.title=bus.name+(groups.includes(bus)?' · 编组效果器':' · 返回效果器');
    view.level.textContent=bus.mute?'静音':dbText(bus.gain_db);
  }
}
