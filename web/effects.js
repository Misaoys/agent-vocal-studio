'use strict';

const fxButton=document.createElement('button');
fxButton.id='showEffects';
fxButton.innerHTML=icon('sliders')+'<span>效果器</span>';
fxButton.title='混音效果器 · F3';
$('arrangeToggle').before(fxButton);

const fxDialog=document.createElement('dialog');
fxDialog.className='studio-fx';
fxDialog.setAttribute('aria-labelledby','studioFxHeading');
fxDialog.innerHTML=`<div class="dialog-heading"><h2 id="studioFxHeading">效果器</h2><button id="studioFxClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="studio-fx-toolbar"><select id="studioFxScope" aria-label="效果器目标"></select><div id="studioFxAddControls"><select id="studioFxKind" aria-label="选择效果器"></select><button id="studioFxAdd" aria-label="添加效果器">${icon('plus')}</button></div></div>
  <div id="studioFxChain"></div>
  <div class="studio-fx-footer"><output id="studioFxMessage" role="status"></output><button id="studioFxSave" class="ghost">应用</button><button id="studioFxPreview" class="primary">应用并试听</button></div>`;
document.body.append(fxDialog);
wireDialog(fxDialog);

let fxCatalog,fxDraft,fxSaving=false,fxMonitorTimer;
const masterScope='master';

function fxParamRow(fields,settings,onChange){
  const row=document.createElement('div');
  row.className='studio-fx-fields';
  for(const [key,name,min,max,step] of fields){
    const label=document.createElement('label');
    label.textContent=name;
    let input;
    if(Array.isArray(min)){
      input=document.createElement('select');
      for(const [value,text] of min)input.add(new Option(text,value));
      input.value=settings[key]??min[0][0];
    }else{
      input=document.createElement('input');
      if(min===undefined){input.type='checkbox';input.checked=settings[key];label.classList.add('fx-check')}
      else{input.type='number';input.min=min;input.max=max;input.step=step;input.value=settings[key]}
    }
    input.dataset.param=key;
    input.disabled=key==='gain_db'&&settings.shape!==undefined&&settings.shape!=='bell';
    input.onchange=()=>{
      if(input.type==='number'&&(!input.checkValidity()||!Number.isFinite(input.valueAsNumber))){input.reportValidity();input.value=settings[key];return}
      settings[key]=input.type==='number'?input.valueAsNumber:input.type==='checkbox'?input.checked:input.value;
      if(key==='peak_mode'&&input.value==='sample_peak')delete settings.peak_mode;
      if(key==='shape'&&settings.shape!=='bell')settings.gain_db=0;
      onChange(key==='shape'||key==='peak_mode');
    };
    if(input.type==='number')input.oninput=()=>{
      if(!input.checkValidity()||!Number.isFinite(input.valueAsNumber))return;
      settings[key]=input.valueAsNumber;
      fxShowChanges();
    };
    label.append(input);
    row.append(label);
  }
  return row;
}

function fxCommand(glyph,label,run){
  const button=document.createElement('button');
  button.innerHTML=icon(glyph);
  button.title=label;
  button.setAttribute('aria-label',label);
  button.onclick=run;
  return button;
}

function fxShowChanges(){
  const operations=fxChanges();
  $('studioFxMessage').textContent=audio.liveMix?'实时监听 · 总线零前瞻':'';
  $('studioFxSave').disabled=!operations.length;
  clearTimeout(fxMonitorTimer);
  if(!fxDraft||!fxDialog.open||fxSaving)return;
  if(fxDraft.master?.peak_mode==='true_peak'||project.master_limiter?.peak_mode==='true_peak'){
    $('studioFxMessage').textContent='真峰值 · 渲染试听';
    if(fxDraft.previewRequested){
      fxDraft.previewRequested=false;
      fxDraft.previewSignature=null;
      void audio.monitorMix(project,rendered?.job_id,{tracks:[]}).catch(()=>{});
    }
    return;
  }
  const activeRender=rendered?.project_id===fxDraft.project_id&&rendered.revision===fxDraft.expected_revision&&rendered.preview_sources?.length;
  if(!window.desktopNative)return;
  if(!audio.liveMix&&(audio.paused||!activeRender)){
    if(operations.length&&!audio.paused)$('studioFxMessage').textContent='应用并试听后可继续实时调整';
    return;
  }
  const settings={tracks:operations.filter(op=>op.op==='update_track').map(op=>({track_id:op.track_id,effects:op.values.effects}))};
  const master=operations.find(op=>op.op==='set_project');
  if(master)Object.assign(settings,master.values);
  const binding=fxDraft;
  const signature=JSON.stringify(settings);
  if(signature===binding.previewSignature||(!operations.length&&!binding.previewSignature))return;
  fxMonitorTimer=setTimeout(async()=>{
    if(fxDraft!==binding||!fxDialog.open||project?.id!==binding.project_id||project.revision!==binding.expected_revision)return;
    binding.previewSignature=signature;
    binding.previewRequested=true;
    try{
      const status=await audio.monitorMix(project,rendered?.job_id,settings);
      if(fxDraft===binding&&fxDialog.open&&!fxSaving&&signature===binding.previewSignature&&status?.live_mix)
        $('studioFxMessage').textContent='实时监听 · 总线零前瞻';
    }catch(error){
      if(fxDraft===binding&&fxDialog.open&&signature===binding.previewSignature){
        binding.previewSignature=null;
        $('studioFxMessage').textContent=error.message;
      }
    }
  },70);
}

function fxRender(){
  const scope=$('studioFxScope').value;
  const output=$('studioFxChain');
  output.replaceChildren();
  $('studioFxAddControls').hidden=scope===masterScope;
  const refresh=redraw=>{if(redraw)fxRender();else fxShowChanges()};
  if(scope===masterScope){
    const id='studioFxMasterEnabled';
    const check=document.createElement('input');
    check.type='checkbox';check.id=id;check.checked=!!fxDraft.master;
    const title=document.createElement('label');
    title.className='studio-fx-master-toggle';title.append(check,document.createTextNode(fxCatalog.master_limiter.label));
    check.onchange=()=>{fxDraft.master=check.checked?structuredClone(fxCatalog.master_limiter.defaults):null;fxRender()};
    output.append(title);
    if(fxDraft.master){
      const fields=fxCatalog.master_limiter.fields.map(([key,label,...domain])=>
        [key,key==='ceiling_db'&&fxDraft.master.peak_mode==='true_peak'?'上限 · dBTP':label,...domain]);
      output.append(fxParamRow(fields,fxDraft.master,refresh));
    }
    fxShowChanges();
    return;
  }
  const chain=fxScopeChain(scope);
  if(!chain)return;
  $('studioFxAdd').disabled=chain.length>=fxCatalog.track_limit;
  for(const [index,settings] of chain.entries()){
    const spec=fxCatalog.effects.find(effect=>effect.kind===settings.kind);
    if(!spec)continue;
    const item=document.createElement('section');item.className='studio-fx-insert';
    item.dataset.bypassed=String(settings.bypass);
    const label=document.createElement('strong');label.textContent=spec.label;
    const bypass=document.createElement('label');bypass.className='fx-check';
    const toggle=document.createElement('input');toggle.type='checkbox';toggle.checked=settings.bypass;
    toggle.onchange=()=>{settings.bypass=toggle.checked;item.dataset.bypassed=String(settings.bypass);fxShowChanges()};
    bypass.append(toggle,document.createTextNode('旁路'));
    const shift=delta=>{const moved=chain.splice(index,1)[0];chain.splice(index+delta,0,moved);fxRender()};
    const up=fxCommand('up','上移效果器',()=>shift(-1));up.disabled=index===0;
    const down=fxCommand('down','下移效果器',()=>shift(1));down.disabled=index===chain.length-1;
    const remove=fxCommand('close','移除效果器',()=>{chain.splice(index,1);fxRender()});
    const header=document.createElement('div');header.className='studio-fx-insert-title';
    header.append(label,bypass,up,down,remove);
    const fields=spec.fields.map(([key,name,...domain])=>key==='source_track_id'
      ? [key,name,fxKeyChoices(scope)] : [key,name,...domain]);
    item.append(header,fxParamRow(fields,settings,refresh));
    output.append(item);
  }
  fxShowChanges();
}

function fxChanges(){
  if(!fxDraft)return [];
  const edits=[];
  for(const [track_id,effects] of fxDraft.tracks){
    if(JSON.stringify(effects)!==fxDraft.original.get(track_id))edits.push({op:'update_track',track_id,values:{effects}});
  }
  const values={};
  if(JSON.stringify(fxDraft.master)!==fxDraft.originalMaster)values.master_limiter=fxDraft.master;
  if(JSON.stringify(fxDraft.buses)!==fxDraft.originalBuses)values.aux_buses=fxDraft.buses;
  if(JSON.stringify(fxDraft.groups)!==fxDraft.originalGroups)values.mix_groups=fxDraft.groups;
  if(Object.keys(values).length)edits.push({op:'set_project',values});
  return edits;
}

function fxScopeChain(scope){
  return fxDraft?.tracks.get(scope)??fxDraft?.buses.find(bus=>'aux:'+bus.id===scope)?.effects
    ??fxDraft?.groups.find(bus=>'group:'+bus.id===scope)?.effects;
}

function fxKeyChoices(scope){
  return project.tracks.filter(track=>track.id!==scope).map(track=>[track.id,track.name]);
}

async function openEffects(busId){
  if(!project||fxDialog.open)return;
  try{
    fxCatalog??=await api('effect_catalog');
    const tracks=new Map(project.tracks.map(track=>[track.id,structuredClone(track.effects||[])]));
    const master=structuredClone(project.master_limiter||null);
    const buses=structuredClone(project.aux_buses||[]);
    const groups=structuredClone(project.mix_groups||[]);
    fxDraft={project_id:project.id,expected_revision:project.revision,tracks,master,buses,groups,
      original:new Map([...tracks].map(([id,items])=>[id,JSON.stringify(items)])),
      originalMaster:JSON.stringify(master),originalBuses:JSON.stringify(buses),originalGroups:JSON.stringify(groups)};
    $('studioFxScope').replaceChildren(...project.tracks.map(track=>new Option(track.name,track.id)),
      ...groups.map(bus=>new Option(bus.name+' · 编组','group:'+bus.id)),
      ...buses.map(bus=>new Option(bus.name+' · 返回','aux:'+bus.id)),new Option('总线',masterScope));
    $('studioFxScope').value=typeof busId==='string'?(groups.some(bus=>bus.id===busId)?'group:':'aux:')+busId:tr().id;
    $('studioFxKind').replaceChildren(...fxCatalog.effects.map(spec=>new Option(spec.label,spec.kind)));
    fxRender();
    fxDialog.showModal();
    fxShowChanges();
  }catch(error){toast(error.message,true)}
}

async function applyEffects(listen){
  if(fxSaving||!fxDraft)return;
  const {project_id,expected_revision}=fxDraft;
  if(project?.id!==project_id||project.revision!==expected_revision){$('studioFxMessage').textContent='工程已更新，请重新打开效果器';return}
  const operations=fxChanges();
  const inputs=[...fxDialog.querySelectorAll('input[type=number]:not(:disabled)')];
  if(inputs.some(input=>!input.checkValidity()||!Number.isFinite(input.valueAsNumber))){$('studioFxMessage').textContent='请检查数值范围';return}
  fxSaving=true;
  clearTimeout(fxMonitorTimer);
  $('studioFxSave').disabled=true;
  $('studioFxPreview').disabled=true;
  try{
    if(operations.length)await edit(operations,{project_id,expected_revision});
    fxDraft.applied=true;
    fxDialog.close();
    if(listen&&(fxDraft.master?.peak_mode==='true_peak'||!audio.liveMix||audio.paused)){
      if(!audio.paused)audio.pause();
      await render(true);
    }
  }catch(error){$('studioFxMessage').textContent=error.message}
  finally{fxSaving=false;$('studioFxPreview').disabled=false;$('studioFxSave').disabled=!fxChanges().length}
}

fxButton.onclick=openEffects;
$('studioFxClose').onclick=()=>fxDialog.close();
const closeEffects=fxDialog.close.bind(fxDialog);
fxDialog.close=()=>{
  if(fxSaving&&!fxDraft?.applied)return;
  clearTimeout(fxMonitorTimer);
  if(fxDraft?.previewRequested&&!fxDraft.applied&&project?.id===fxDraft.project_id&&project.revision===fxDraft.expected_revision){
    fxDraft.previewRequested=false;
    void audio.monitorMix(project,rendered?.job_id,{tracks:[]}).catch(()=>{
      if(audio.liveMix)toast('试听参数未恢复，请重新播放',true)
    });
  }
  closeEffects();
};
$('studioFxScope').onchange=fxRender;
$('studioFxAdd').onclick=()=>{
  const scope=$('studioFxScope').value;
  const chain=fxScopeChain(scope);
  const spec=fxCatalog.effects.find(effect=>effect.kind===$('studioFxKind').value);
  if(!chain||!spec||chain.length>=fxCatalog.track_limit)return;
  const effect=structuredClone(spec.defaults);
  if(effect.kind==='ducker'){
    const trigger=fxKeyChoices(scope)[0];
    if(!trigger){$('studioFxMessage').textContent='先添加另一条触发音轨';return}
    effect.source_track_id=trigger[0];
  }
  chain.push(effect);
  fxRender();
};
$('studioFxSave').onclick=()=>void applyEffects(false);
$('studioFxPreview').onclick=()=>void applyEffects(true);
