'use strict';
// Original, lightweight SVG interface icons. No remote assets or icon fonts.
const iconPaths={
  metronome:'M7 3h6l4 14H3ZM10 13l5-7M7 14h6',
  sliders:'M3 5h14M3 10h14M3 15h14M7 3v4M13 8v4M8 13v4',
  record:'M10 3a7 7 0 1 0 0 14a7 7 0 1 0 0-14ZM10 7a3 3 0 1 0 0 6a3 3 0 1 0 0-6Z',
  play:'M7 4l10 6-10 6Z',pause:'M7 4v12M13 4v12',stop:'M5 5h10v10H5Z',
  loop:'M4 6h10a3 3 0 0 1 3 3v1M14 3l3 3-3 3M16 14H6a3 3 0 0 1-3-3v-1M6 17l-3-3 3-3',
  plus:'M10 4v12M4 10h12',minus:'M4 10h12',
  folder:'M2.5 5.5h6l2 2h7v9h-15Z',import:'M10 2v10M6 8l4 4 4-4M3 12v5h14v-5',export:'M10 13V3M6 7l4-4 4 4M3 12v5h14v-5',
  undo:'M7 4L3 8l4 4M3 8h8a5 5 0 0 1 0 10',redo:'M13 4l4 4-4 4M17 8H9a5 5 0 0 0 0 10',
  down:'M10 3v14M5 12l5 5 5-5',up:'M10 17V3M5 8l5-5 5 5',copy:'M7 3h10v11H7ZM3 7v10h10',
  grid:'M3 3h14v14H3ZM3 8h14M8 3v14',lyrics:'M3 4h14M3 9h9M3 14h6M14 12v6M11 15h6',
  grip:'M6 6h8M6 10h8M6 14h8',
  flag:'M5 17V3h10l-2 3 2 3H5',
  speaker:'M3 8h4l4-4v12l-4-4H3Zm11-1a4 4 0 0 1 0 6M16 5a7 7 0 0 1 0 10',
  book:'M10 5C7 3 4 3 2 4v12c2-1 5-1 8 1 3-2 6-2 8-1V4c-2-1-5-1-8 1ZM10 5v12',
  plan:'M5 3h10v14H5ZM8 7h4M8 10h4M8 13h3',check:'M3 10l4 4 10-10',review:'M8.5 3a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM13 13l4 4',
  follow:'M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14ZM10 7v6M7 10h6',
  close:'M5 5l10 10M15 5L5 15',wave:'M2 9v2M6 5v10M10 2v16M14 5v10M18 8v4',
  slip:'M3 5v10M17 5v10M6 10h8M8 8l-2 2 2 2M12 8l2 2-2 2',
  midi:'M3 4h14v12H3ZM6 4v7M10 4v7M14 4v7',pitch:'M2 13l4-6 4 4 4-7 4 4',velocity:'M4 14V9M8 14V5M12 14V7M16 14V3'
};
function icon(name){return `<svg class="ui-icon" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${iconPaths[name]||iconPaths.wave}"/></svg>`}
function labelButton(id,name,text=''){const b=$(id);if(b)b.innerHTML=icon(name)+(text?`<span>${text}</span>`:'')}
const compactEngine=document.querySelector('.engine-badge');if(compactEngine)document.querySelector('.statusbar').insertBefore(compactEngine,document.querySelector('.connection'));
document.querySelector('.session-head')?.remove();document.querySelector('.brandmark').innerHTML=icon('wave');document.querySelector('.subbrand').textContent='STUDIO';
const labels=[['newProject','plus'],['addTrack','plus'],['importBtn','import','导入'],['exportProject','folder','工程包'],['exportWav','export','导出'],['stop','stop'],['play','play'],['loop','loop'],['undo','undo'],['redo','redo'],['batchLyrics','lyrics','歌词'],['transposeDown','down'],['transposeUp','up'],['quantizeNotes','grid','量化'],['duplicateNotes','copy'],['zoomOut','minus'],['zoomIn','plus'],['followAgent','follow','跟随'],['showDesign','plan','设计'],['reviewSong','review','复核'],['showKnowledge','book','知识'],['closeKnowledge','close'],['exportMidi','midi','MIDI'],['pitchTab','pitch','音高'],['velocityTab','velocity','力度']];for(const [id,name,text]of labels)labelButton(id,name,text);
$('saveState').textContent='已保存';document.querySelector('.sidebar-footer small')?.remove();
audio.addEventListener('play',()=>labelButton('play','pause'));for(const event of ['pause','ended'])audio.addEventListener(event,()=>labelButton('play','play'));
const projectIconObserver=new MutationObserver(()=>{for(const el of $('projects').querySelectorAll('.folder:not([data-icon])')){el.dataset.icon='true';el.innerHTML=icon('folder')}});projectIconObserver.observe($('projects'),{childList:true,subtree:true});
for(const el of $('projects').querySelectorAll('.folder')){el.dataset.icon='true';el.innerHTML=icon('folder')}
const closeLyrics=$('lyricsDialog').querySelector('[aria-label="关闭"]');if(closeLyrics)closeLyrics.innerHTML=icon('close');
labelButton('pitchJoin','pitch','滑音');
$('pitchJoin').title='单个音符处理起音或尾音，两个相接音符可做衔接';
labelButton('pitchAuditionStop','stop');
$('pitchJoinDialog').querySelector('[aria-label="关闭"]').innerHTML=icon('close');
// Sticky ruler and keyboard use the same musical coordinates as the editable canvas.
const stickyRuler=document.createElement('canvas'),stickyKeys=document.createElement('canvas');stickyRuler.className='sticky-ruler';stickyKeys.className='sticky-keys';$('rollViewport').append(stickyKeys,stickyRuler);
function fixedChrome(){if(!project)return;const view=$('rollViewport'),w=view.clientWidth,h=(maxPitch-minPitch+1)*rowH;canvasSize(stickyRuler,w,rulerH);canvasSize(stickyKeys,keyW,h);stickyRuler.style.transform=`translate(${view.scrollLeft}px,${view.scrollTop}px)`;stickyKeys.style.transform=`translateX(${view.scrollLeft}px)`;const c=stickyRuler.getContext('2d'),k=stickyKeys.getContext('2d');stickyKeys.style.top=rulerH+'px';timelineRuler(c,w,view.scrollLeft);const names=['C','C♯','D','D♯','E','F','F♯','G','G♯','A','A♯','B'];k.font='9px Studio Inter, Studio Han, sans-serif';for(let p=minPitch;p<=maxPitch;p++){const y=(maxPitch-p)*rowH,black=[1,3,6,8,10].includes(p%12);k.fillStyle=black?'#212125':'#2b2b31';k.fillRect(0,y,keyW-1,rowH-1);k.fillStyle=p%12===0?'#cfcfd2':'#a3a3a7';k.fillText(names[p%12]+(Math.floor(p/12)-1),p%12===0?13:19,y+14)}}
const beforeFixedDraw=draw;draw=function(){beforeFixedDraw();fixedChrome()};$('rollViewport').addEventListener('scroll',fixedChrome);
