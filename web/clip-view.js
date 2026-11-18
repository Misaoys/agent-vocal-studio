'use strict';

const clipRate=48000;
const clipWaveforms=new Map();
const clipAssetFrames=new Map();
const clipWaveRequests=new Set();
const clipWaveFailures=new Map();
const trackColors=['#529bff','#f0b53f','#30d5a2','#b483ff','#f87587','#29c9e8','#b3d84c','#ff9858'];

function trackColor(track){
  const index=project?.tracks.findIndex(candidate=>candidate.id===track?.id)??-1;
  return trackColors[track?.color_slot??Math.max(0,index)%trackColors.length];
}

function scoreNotePalette(color,selected){
  const channels=(color.match(/[0-9a-f]{2}/gi)||[]).map(value=>parseInt(value,16));
  const fill=selected?channels:channels.map((value,index)=>Math.round(value*.81+[16,16,18][index]*.19));
  const light=fill.reduce((sum,value,index)=>{
    const srgb=value/255;
    return sum+[.2126,.7152,.0722][index]*(srgb<=.04045?srgb/12.92:((srgb+.055)/1.055)**2.4);
  },0);
  return {
    fill:'#'+fill.map(value=>value.toString(16).padStart(2,'0')).join(''),
    ink:light>=Math.sqrt(.0525)-.05?'#000000':'#ffffff'
  };
}

function isAudioTrack(track){
  return !!track&&(Array.isArray(track.clips)||!!track.audio);
}

function trackClips(track){
  if(Array.isArray(track?.clips))return track.clips;
  const legacy=track?.audio;
  return legacy?[{id:'legacy',asset:legacy.asset,name:track.name,start:legacy.start,offset:0,
    frames:Math.round(legacy.seconds*clipRate),gain_db:0,fade_in:0,fade_out:0}]:[];
}

function clipStartFrame(clip){
  return Math.round(beatSeconds(clip.start)*clipRate);
}

function clipEndBeat(clip){
  return clip.beat_length?clip.start+clip.beat_length:secondsBeat((clipStartFrame(clip)+clip.frames)/clipRate);
}
function clipEndFrame(clip){
  return clip.beat_length?Math.round(beatSeconds(clipEndBeat(clip))*clipRate):clipStartFrame(clip)+clip.frames;
}
function clipBeatAtSource(clip,frame,extend=false){
  return clip.beat_length?clip.start+mapClipWarp(clip,'frame','beat',frame,extend):secondsBeat((clipStartFrame(clip)+frame)/clipRate);
}
function clipSourceAtBeat(clip,beat,extend=false){
  return clip.beat_length?Math.round(mapClipWarp(clip,'beat','frame',beat-clip.start,extend)):Math.round(beatSeconds(beat)*clipRate)-clipStartFrame(clip);
}
function mapClipWarp(clip,input,output,value,extend=false){
  const points=clip.warp_points||[];
  const index=points.length?upperTimeline(points,input,value):-1;
  const earlier=points[index];
  const a=earlier&&earlier[input]<=value?earlier:{frame:0,beat:0};
  const b=points[a===earlier?index+1:0]||{frame:clip.frames,beat:clip.beat_length};
  const ratio=(value-a[input])/(b[input]-a[input]);
  return a[output]+(b[output]-a[output])*(extend?ratio:Math.max(0,Math.min(1,ratio)));
}

function fadeGain(progress,curve){
  return curve==='equal_power'?Math.sin(Math.PI*.5*progress):progress;
}

function rememberWaveformValue(cache,key,value){
  cache.delete(key);
  cache.set(key,value);
  if(cache.size>128)cache.delete(cache.keys().next().value);
}

function clipWaveformKey(clip){
  return JSON.stringify([clip.asset,clip.offset,clip.frames]);
}

function visibleWaveformBins(clip,source){
  const stride=source.frames/source.peaks.length;
  const begin=clip.offset-source.offset;
  return {peaks:source.peaks,stride,begin,
    first:Math.floor(begin/stride),
    end:Math.min(source.peaks.length,Math.ceil((begin+clip.frames)/stride))};
}

function clipWaveform(track,clip,repaint){
  const key=clipWaveformKey(clip),exact=clipWaveforms.get(key);
  if(exact){
    rememberWaveformValue(clipWaveforms,key,exact);
    return visibleWaveformBins(clip,exact);
  }
  const request={project_id:project.id,expected_revision:project.revision,track_id:track.id,clip_id:clip.id};
  const binding=JSON.stringify([key,request.project_id,request.expected_revision,track.id,clip.id]);
  const failed=clipWaveFailures.get(binding);
  const requestAllowed=!failed||(failed.attempts===1&&performance.now()>=failed.retryAt);
  if(requestAllowed&&!clipWaveRequests.has(key)&&clipWaveRequests.size<6){
    clipWaveRequests.add(key);
    api('audio_waveform',request).then(data=>{
      if(data.source_offset!==clip.offset||data.frames!==clip.frames||!data.peaks?.length)
        throw new Error('Waveform window changed');
      clipWaveFailures.delete(binding);
      if(Number.isInteger(data.asset_frames))rememberWaveformValue(clipAssetFrames,clip.asset,data.asset_frames);
      rememberWaveformValue(clipWaveforms,key,{asset:clip.asset,offset:clip.offset,frames:clip.frames,
        assetFrames:data.asset_frames,peaks:data.peaks});
    }).catch(()=>{
      const attempts=(failed?.attempts||0)+1;
      rememberWaveformValue(clipWaveFailures,binding,{attempts,retryAt:performance.now()+350});
      if(attempts===1)setTimeout(repaintViews,360);
    }).finally(()=>{
      clipWaveRequests.delete(key);
      repaintViews();
    });
  }
  let covering=null;
  for(const source of clipWaveforms.values()){
    if(source.asset===clip.asset&&source.offset<=clip.offset&&
      source.offset+source.frames>=clip.offset+clip.frames&&(!covering||source.frames<covering.frames))covering=source;
  }
  return covering?visibleWaveformBins(clip,covering):null;

  function repaintViews(){
    repaint?.();
    document.dispatchEvent(new Event('clip-waveform-ready'));
  }
}

function paintClipWave(c,track,clip,xForBeat,top,height,minX,maxX,repaint){
  const left=xForBeat(clip.start),right=xForBeat(clipEndBeat(clip));
  if(right<minX||left>maxX)return;
  const width=Math.max(2,right-left);
  const color=trackColor(track);
  c.fillStyle=color+'52';c.fillRect(left,top,width,height);
  c.fillStyle=color;c.fillRect(left,top,2,height);
  if(width>35){
    c.font='10px Studio Inter, Studio Han, sans-serif';c.fillStyle='#f0f0f0';
    c.fillText(clip.name,Math.max(left,minX)+6,top+13,width-10);
  }
  if(width<8)return;
  const waveform=clipWaveform(track,clip,repaint);
  if(waveform){
    const {first,end,peaks,stride,begin}=waveform;
    const centre=top+height*.69,amplitude=height*.24;
    c.strokeStyle=color;c.beginPath();
    for(let i=first;i<end;i++){
      const frame=Math.max(0,Math.min(clip.frames,(i+.5)*stride-begin));
      const x=xForBeat(clipBeatAtSource(clip,frame));
      if(x<minX||x>maxX)continue;
      const [lo,hi]=peaks[i];
      c.moveTo(x,centre-Math.min(1,hi)*amplitude);
      c.lineTo(x,centre-Math.max(-1,lo)*amplitude);
    }
    c.stroke();
  }
  c.strokeStyle=color;c.beginPath();
  for(const [count,offset,beats,curve,reverse] of [
    [clip.fade_in,0,clip.fade_in_beats,clip.fade_in_curve,false],
    [clip.fade_out,clip.frames-clip.fade_out,clip.fade_out_beats,clip.fade_out_curve,true]
  ]){
    if(!count&&!beats)continue;
    const end=clipEndBeat(clip);
    const [first,last]=beats==null?
      [clipBeatAtSource(clip,offset),clipBeatAtSource(clip,offset+count)]:
      reverse?[end-beats,end]:[clip.start,clip.start+beats];
    const begin=beatSeconds(first),finish=beatSeconds(last);
    for(let i=0;i<=8;i++){
      const t=i/8,gain=fadeGain(reverse?1-t:t,curve);
      const x=xForBeat(secondsBeat(begin+(finish-begin)*t)),y=top+height-(height-2)*gain;
      if(i===0)c.moveTo(x,y);else c.lineTo(x,y);
    }
  }
  c.stroke();
  if(clip.warp_points?.length){
    c.fillStyle='#e6e6ea';
    for(const marker of clip.warp_points){
      const x=xForBeat(clip.start+marker.beat);
      if(x>=minX&&x<=maxX)c.fillRect(x,top,1.5,5);
    }
  }
}
