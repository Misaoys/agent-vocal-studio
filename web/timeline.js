'use strict';
// Mirrors the Rust timeline formulas; integration tests compare both implementations.
const timelineClocks=new WeakMap();
function upperTimeline(items,key,value){let a=0,b=items.length;while(a<b){const m=(a+b)>>>1;if(items[m][key]<=value)a=m+1;else b=m}return Math.max(0,a-1)}
function barLength(m){return m[0]*4/m[1]}
function timeIntegral(delta,bpm,slope){return Math.abs(slope)<1e-10?60*delta/bpm:60/slope*Math.log1p(slope*delta/bpm)}
class TimelineClock{
 constructor(p){this.segments=[];this.meters=[{beat:0,bar:1,meter:p.meter}];let beat=0,seconds=0,bpm=p.bpm;
  for(const point of p.tempo_map||[]){const slope=point.curve==='linear'?(point.bpm-bpm)/(point.beat-beat):0;this.segments.push({beat,seconds,bpm,slope});seconds+=timeIntegral(point.beat-beat,bpm,slope);beat=point.beat;bpm=point.bpm}this.segments.push({beat,seconds,bpm,slope:0});
  for(const point of p.meter_map||[]){const previous=this.meters.at(-1),bars=Math.max(1,Math.ceil((point.beat-previous.beat)/barLength(previous.meter)-1e-9));this.meters.push({beat:point.beat,bar:previous.bar+bars,meter:point.meter})}
 }
 seconds(beat){const s=this.segments[upperTimeline(this.segments,'beat',beat)];return s.seconds+timeIntegral(Math.max(0,beat-s.beat),s.bpm,s.slope)}
 beat(seconds){const s=this.segments[upperTimeline(this.segments,'seconds',seconds)],time=Math.max(0,seconds-s.seconds);return s.beat+(Math.abs(s.slope)<1e-10?time*s.bpm/60:s.bpm/s.slope*Math.expm1(time*s.slope/60))}
 tempo(beat){const s=this.segments[upperTimeline(this.segments,'beat',beat)];return s.bpm+s.slope*Math.max(0,beat-s.beat)}
 position(beat){const m=this.meters[upperTimeline(this.meters,'beat',beat+1e-9)],n=Math.floor(Math.max(0,beat-m.beat)/barLength(m.meter)+1e-9),start=m.beat+n*barLength(m.meter),unit=Math.max(0,beat-start)*m.meter[1]/4;return {bar:m.bar+n,beat:Math.floor(unit)+1,fraction:unit-Math.floor(unit),meter:m.meter,start}}
 bars(limit){const out=[];for(let i=0;i<this.meters.length;i++){const m=this.meters[i],end=Math.min(limit+1e-8,this.meters[i+1]?.beat??Infinity),length=barLength(m.meter);for(let beat=m.beat,index=0;beat<end;beat=m.beat+(++index)*length){out.push({beat,bar:m.bar+index,meter:m.meter,change:i>0&&index===0})}}return out}
}
function clockFor(p=project){if(!timelineClocks.has(p))timelineClocks.set(p,new TimelineClock(p));return timelineClocks.get(p)}
function beatSeconds(beat,p=project){return clockFor(p).seconds(beat)}
function secondsBeat(seconds,p=project){return clockFor(p).beat(seconds)}
function timelineRuler(c,w,offset=0){
 if(!project)return;c.fillStyle='#1b1b1d';c.fillRect(0,0,w,rulerH);c.font='10px Studio Inter, Studio Han, sans-serif';
 for(const b of clockFor().bars(beats())){const x=beatX(b.beat)-offset;if(x<keyW||x>w)continue;c.fillStyle=b.change?'#c1c1c6':'#a7a7ac';c.fillText(String(b.bar)+(b.change?' · '+b.meter.join('/'):' '),x+7,13)}
 const points=[{beat:0,bpm:project.bpm},...(project.tempo_map||[])];let last=-Infinity;
 for(const p of points){const x=beatX(p.beat)-offset;if(x<keyW||x>w||x-last<46)continue;last=x;c.fillStyle=p.curve==='linear'?'#c7c7ce':'#a5a5aa';c.fillText((p.curve==='linear'?'~ ':'')+Number(p.bpm.toFixed(2)),x+7,29);c.fillRect(x,23,3,3)}
 c.fillStyle='#1b1b1d';c.fillRect(0,0,keyW,rulerH);c.fillStyle='#aeaeb3';c.fillText('小节',14,13);c.fillText('BPM',14,29);c.strokeStyle='#303034';c.beginPath();c.moveTo(0,rulerH-.5);c.lineTo(w,rulerH-.5);c.stroke();
}
