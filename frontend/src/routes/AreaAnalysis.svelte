<script lang="ts">
  // The "Analytic Browser" tab: window-at-a-time view of sequence alignments
  // on a chosen chromosome, rendered as stacked horizontal bars. Heavy
  // caching (color, bar geometry, stacked tracks) plus an IntersectionObserver
  // gate so nothing computes until the tab is actually visible.

  import { onMount, onDestroy } from 'svelte';
  import type { BackendMatch, ChromosomeInfo } from '$lib/bincodeDecoder';
  import type { FileData } from '$lib/types';
  import { searchStore } from '$lib/searchStore';
  import { areaFltState } from '$lib/filterStateStore';
  import {
    fetchChrRecs,
    fetchSeqLocations,
    type WireAreaRecord,
    type ChrRecsResponse,
    type SeqLocation,
  } from '$lib/queryClient';

  // All data flows in from +page.svelte. Component fetches chromosome
  // data via sessId once isQueryable && isVisible.
  export let matches: BackendMatch[] = [];
  /** One entry per genome (2–3). */
  export let files: FileData[] = [];
  /** Flat file_index → genome index. */
  export let fileToGen: number[] = [];
  /** Per-genome chromosome info (ref_contig_id + ref_len). */
  export let chrInfo: ChromosomeInfo[][] = [];

  /** Session id for backend queries. */
  export let sessId: string | null = null;

  /** True once match phase has completed. Fetches gated on this. */
  export let isQueryable: boolean = false;

  // IntersectionObserver-driven lazy init.
  let isVisible = false;
  let containerEl: HTMLElement;
  let isInit = false;

  // Selected genome indices (persisted in areaFltState.selFiles).
  let selGens: number[] = [];
  let selChr = 1;
  let winSize = 100000;
  let curWinIdx = 0;
  let hoverRec: any = null;

  // Chromosome-record cache. Key: `"{sorted genome indices}|{chromosome}"`.
  // Invalidated when selGens or sessId changes.

  let chrRecCache = new Map<string, ChrRecsResponse>();
  let chrRecsAbort: AbortController | null = null;
  let chrRecsLdg = false;
  let chrRecsActKey: string | null = null;
  let chrRecs: WireAreaRecord[] = [];
  let chrRefLenFetched: number = 0;

  function cacheKey(genomes: number[], chr: number): string {
    return `${[...genomes].sort((a, b) => a - b).join(',')}|${chr}`;
  }

  async function reloadChrRecs() {
    if (!sessId || !isQueryable || !isVisible) {
      chrRecs = [];
      chrRefLenFetched = 0;
      chrRecsActKey = null;
      return;
    }
    if (selGens.length === 0) {
      chrRecs = [];
      chrRefLenFetched = 0;
      chrRecsActKey = null;
      return;
    }

    const key = cacheKey(selGens, selChr);

    const cached = chrRecCache.get(key);
    if (cached) {
      chrRecs = cached.records;
      chrRefLenFetched = cached.chromosome_ref_len;
      chrRecsActKey = key;
      clearCaches();
      return;
    }

    if (chrRecsAbort) {
      chrRecsAbort.abort();
    }
    chrRecsAbort = new AbortController();
    const signal = chrRecsAbort.signal;

    const chipTimer = setTimeout(() => { chrRecsLdg = true; }, 200);

    try {
      const resp = await fetchChrRecs(sessId, {
        genomes: selGens,
        chr: selChr,
        signal,
      });
      if (resp === undefined) return;

      const currentKey = cacheKey(selGens, selChr);
      if (key !== currentKey) return;

      chrRecCache.set(key, resp);
      chrRecs = resp.records;
      chrRefLenFetched = resp.chromosome_ref_len;
      chrRecsActKey = key;
      clearCaches();
    } catch (err) {
      console.error('Failed to fetch chromosome records:', err);
      chrRecs = [];
      chrRefLenFetched = 0;
    } finally {
      clearTimeout(chipTimer);
      chrRecsLdg = false;
    }
  }

  // Refetch when any dep changes. Stale-key check + AbortController
  // handle in-flight switches.
  $: {
    void sessId;
    void isQueryable;
    void isVisible;
    void selGens;
    void selChr;
    reloadChrRecs();
  }

  // Invalidate cache on genome-selection change.
  let lastGensKey = '';
  $: {
    const gk = [...selGens].sort((a, b) => a - b).join(',');
    if (gk !== lastGensKey) {
      lastGensKey = gk;
      chrRecCache.clear();
      chrRecsActKey = null;
    }
  }

  // Invalidate cache on session change.
  let lastSessId: string | null = null;
  $: {
    if (sessId !== lastSessId) {
      lastSessId = sessId;
      chrRecCache.clear();
      chrRecsActKey = null;
    }
  }

  // Sequence-location cache for the overview search.

  let seqLocsCache = new Map<number, SeqLocation[]>();
  let seqLocsAbort: AbortController | null = null;

  /** Locations for currently-searched sequence, or null. */
  let activeSeqLocs: SeqLocation[] | null = null;

  async function loadSeqLocs(seqId: number) {
    if (!sessId || !isQueryable) {
      activeSeqLocs = null;
      return;
    }
    const cached = seqLocsCache.get(seqId);
    if (cached) {
      activeSeqLocs = cached;
      return;
    }
    if (seqLocsAbort) seqLocsAbort.abort();
    seqLocsAbort = new AbortController();
    try {
      const resp = await fetchSeqLocations(sessId, {
        qry: seqId,
        genomes: selGens,
        signal: seqLocsAbort.signal,
      });
      if (!resp) return;
      seqLocsCache.set(seqId, resp.locations);

      const currentRaw = submittedQry.trim();
      if (currentRaw !== '') {
        const current = parseInt(currentRaw, 10);
        if (!Number.isNaN(current) && current === seqId) {
          activeSeqLocs = resp.locations;
        }
      }
    } catch (err) {
      console.error('Failed to fetch sequence locations:', err);
      activeSeqLocs = null;
    }
  }

  $: if (isSrch && submittedQry) {
    const id = parseInt(submittedQry);
    if (!Number.isNaN(id)) loadSeqLocs(id);
  } else {
    activeSeqLocs = null;
  }

  // Invalidate seq-locations cache on genome change.
  let lastLocGensKey = '';
  $: {
    const gk = [...selGens].sort((a, b) => a - b).join(',');
    if (gk !== lastLocGensKey) {
      lastLocGensKey = gk;
      seqLocsCache.clear();
      if (seqLocsAbort) {
        seqLocsAbort.abort();
        seqLocsAbort = null;
      }
      if (isSrch && submittedQry) {
        const id = parseInt(submittedQry);
        if (!Number.isNaN(id)) loadSeqLocs(id);
      }
    }
  }

  let lastLocSessId: string | null = null;
  $: {
    if (sessId !== lastLocSessId) {
      lastLocSessId = sessId;
      seqLocsCache.clear();
      if (seqLocsAbort) {
        seqLocsAbort.abort();
        seqLocsAbort = null;
      }
    }
  }


  let srchQry = '';
  let submittedQry = '';

  /** Window indices containing at least one hit for the searched seq. */
  let fltWins: number[] = [];
  let isSrch = false;

  let shouldRerun = false;

  let ovPanelOpen = false;

  /** One dot (hit cluster) on a chromosome line. */
  interface OvDot {
    /** Fractional position along the chromosome line (0..1). */
    xFrac: number;
    /** Window index a click would land on. */
    estWin: number;
  }

  interface ChrLine {
    chrId: number;
    refLen: number;
    /** 12 tick markers (start + 10 intermediate + end) in bp. */
    markers: number[];
    dots: OvDot[];
  }

  function buildChrLines(
    genIdx: number,
    chrInfoForGen: ChromosomeInfo[],
    seqId: number | null,
    _selGens: number[],
  ): ChrLine[] {
    const sorted = [...chrInfoForGen].sort((a, b) => a.ref_contig_id - b.ref_contig_id);

    const genIsSel = _selGens.includes(genIdx);

    const locs: SeqLocation[] = activeSeqLocs ?? [];

    return sorted.map(chr => {
      const refLen = chr.ref_len;
      const numMarkers = 12;
      const markers: number[] = [];
      for (let i = 0; i < numMarkers; i++) {
        markers.push(Math.round((refLen * i) / (numMarkers - 1)));
      }

      let dots: OvDot[] = [];

      if (seqId !== null) {
        let rangeMin = 0;
        if (genIsSel && chr.ref_contig_id === selChr) {
          rangeMin = Math.floor(chrRange.min);
        }

        const totWinsForChr = Math.max(
          1,
          Math.ceil(Math.max(0, refLen - rangeMin) / winSize),
        );
        const windowedSpan = totWinsForChr * winSize;

        const hitWins = new Set<number>();
        for (const loc of locs) {
          if (loc.genome_index !== genIdx) continue;
          if (loc.ref_contig_id !== chr.ref_contig_id) continue;

          const relStart = Math.max(0, loc.ref_start_pos - rangeMin);
          const relEnd   = Math.max(0, loc.ref_end_pos   - rangeMin);
          const startWin = Math.min(
            totWinsForChr - 1,
            Math.floor(relStart / winSize),
          );
          const endWin = Math.min(
            totWinsForChr - 1,
            Math.floor(relEnd / winSize),
          );
          for (let w = startWin; w <= endWin; w++) {
            if (w >= 0) hitWins.add(w);
          }
        }

        for (const winIdx of hitWins) {
          const winCenterRel = (winIdx + 0.5) * winSize;
          const xFrac = windowedSpan > 0
            ? Math.min(1, winCenterRel / windowedSpan)
            : 0;

          dots.push({ xFrac, estWin: winIdx });
        }

        dots.sort((a, b) => a.xFrac - b.xFrac);
      }

      return {
        chrId: chr.ref_contig_id,
        refLen,
        markers,
        dots,
      };
    });
  }

  $: ovData = ovPanelOpen
    ? buildOvData(isSrch, submittedQry, activeSeqLocs, chrInfo, files, fileToGen, winSize, selGens)
    : [];

  function buildOvData(
    _isSrch: boolean,
    _submittedQry: string,
    _activeSeqLocs: SeqLocation[] | null,
    _chrInfo: ChromosomeInfo[][],
    _files: FileData[],
    _fileToGen: number[],
    _winSize: number,
    _selGens: number[],
  ): { genName: string; genColor: string; lines: ChrLine[] }[] {
    const searchSeqId = (isSrch && submittedQry)
      ? parseInt(submittedQry)
      : null;
    const parsedSeqId = (searchSeqId !== null && !isNaN(searchSeqId)) ? searchSeqId : null;

    return chrInfo.map((ci, gi) => ({
      genName: files[gi]?.name ?? `Genome ${gi}`,
      genColor: files[gi]?.color ?? '#888',
      lines: buildChrLines(gi, ci, parsedSeqId, _selGens),
    }));
  }

  async function navFromDot(chrId: number, estWin: number) {
    selChr = chrId;

    const key = cacheKey(selGens, chrId);
    let newChrRecs: WireAreaRecord[] = [];
    let newChrRange = { min: 0, max: 100000 };

    const cached = chrRecCache.get(key);
    if (cached) {
      newChrRecs = cached.records;
      newChrRange = getChrRange(newChrRecs);
    } else if (sessId && isQueryable) {
      try {
        const resp = await fetchChrRecs(sessId, {
          genomes: selGens,
          chr: chrId,
        });
        if (resp) {
          chrRecCache.set(key, resp);
          newChrRecs = resp.records;
          newChrRange = getChrRange(newChrRecs);
        }
      } catch (err) {
        console.error('navFromDot fetch failed:', err);
      }
    }

    let targetWin = estWin;
    if (isSrch && submittedQry) {
      const seqId = parseInt(submittedQry);
      if (!isNaN(seqId)) {
        fltWins = findWinsWithSeq(seqId, newChrRecs, newChrRange, winSize);

        if (fltWins.length > 0) {
          let nearest = fltWins[0];
          let bestDist = Math.abs(nearest - estWin);
          for (const w of fltWins) {
            const d = Math.abs(w - estWin);
            if (d < bestDist) {
              bestDist = d;
              nearest = w;
            }
          }
          targetWin = nearest;
        }
      }
    }

    curWinIdx = targetWin;

    areaFltState.update(state => ({
      ...state,
      selChr: chrId,
      curWinIdx: targetWin,
    }));

    clearCaches();
  }

  let compPanelOpen = false;

  interface SeqComp {
    /** Sequences present in ALL selected genomes inside the window. */
    shared: number[];
    /** Per-genome breakdown of sequences not shared with all. */
    uniqPerGen: { genIdx: number; genName: string; genColor: string; seqIds: number[] }[];
    /** Total distinct sequence ids across all genomes in this window. */
    totUniq: number;
  }

  function buildWinComp(
    _records: WireAreaRecord[],
    _selGens: number[],
    _winStart: number,
    _winEnd: number,
    _files: FileData[],
  ): SeqComp {
    if (_selGens.length < 2) {
      return { shared: [], uniqPerGen: [], totUniq: 0 };
    }

    const genSeqs = new Map<number, Set<number>>();
    for (const gi of _selGens) {
      genSeqs.set(gi, new Set());
    }

    for (const record of _records) {
      const gi = record.genome_index;
      if (!genSeqs.has(gi)) continue;
      if (record.ref_end_pos >= _winStart && record.ref_start_pos <= _winEnd) {
        genSeqs.get(gi)!.add(record.qry_contig_id);
      }
    }

    const genSets = _selGens.map(gi => genSeqs.get(gi)!);
    const allSeqs = new Set<number>();
    for (const s of genSets) {
      for (const id of s) allSeqs.add(id);
    }

    const shared: number[] = [];
    const sharedSet = new Set<number>();
    for (const id of allSeqs) {
      if (genSets.every(s => s.has(id))) {
        shared.push(id);
        sharedSet.add(id);
      }
    }
    shared.sort((a, b) => a - b);

    const uniqPerGen = _selGens.map(gi => {
      const uniq = Array.from(genSeqs.get(gi)!)
        .filter(id => !sharedSet.has(id))
        .sort((a, b) => a - b);
      return {
        genIdx: gi,
        genName: _files[gi]?.name ?? `Genome ${gi}`,
        genColor: _files[gi]?.color ?? '#888',
        seqIds: uniq,
      };
    });

    return {
      shared,
      uniqPerGen,
      totUniq: allSeqs.size,
    };
  }

  $: winComp = compPanelOpen
    ? buildWinComp(chrRecs, selGens, winStart, winEnd, files)
    : { shared: [], uniqPerGen: [], totUniq: 0 } as SeqComp;

  // Color cache: stable HSL per sequence id.
  const colorCache = new Map<number, string>();

  function seqColor(seqId: number): string {
    if (colorCache.has(seqId)) {
      return colorCache.get(seqId)!;
    }
    const hue = (seqId * 137.508) % 360;
    const color = `hsl(${hue}, 70%, 60%)`;
    colorCache.set(seqId, color);
    return color;
  }

  // Bar geometry cache.
  interface CachedBar {
    record: any;
    startX: number;
    endX: number;
    width: number;
    color: string;
    key: string;
  }

  let cachedBars: CachedBar[][] = [];
  let lastWinStart = -1;
  let lastWinEnd = -1;
  let lastFltRecs: any[] = [];

  // Stacked-bar memoisation.
  let lastStackIn: {
    records: any[];
    winStart: number;
    winEnd: number;
  } | null = null;
  let cachedStacked: any[][] = [];

  function getCachedStacked(records: any[], winStart: number, winEnd: number): any[][] {
    if (
      lastStackIn &&
      lastStackIn.winStart === winStart &&
      lastStackIn.winEnd === winEnd &&
      lastStackIn.records === records
    ) {
      return cachedStacked;
    }

    const stacked = stackBars(records, winStart, winEnd);

    lastStackIn = { records, winStart, winEnd };
    cachedStacked = stacked;

    return stacked;
  }

  function genCachedBars(
    stacked: any[][],
    winStart: number,
    winEnd: number,
    winSize: number
  ): CachedBar[][] {
    if (
      lastWinStart === winStart &&
      lastWinEnd === winEnd &&
      stacked === cachedStacked
    ) {
      return cachedBars;
    }

    const newCache: CachedBar[][] = [];

    for (let trackIdx = 0; trackIdx < stacked.length; trackIdx++) {
      const track = stacked[trackIdx];
      const cachedTrack: CachedBar[] = [];

      for (let recIdx = 0; recIdx < track.length; recIdx++) {
        const record = track[recIdx];
        const startX = posToX(record.ref_start_pos, winStart, winSize);
        const endX = posToX(record.ref_end_pos, winStart, winSize);
        const width = endX - startX;
        const color = seqColor(record.qry_contig_id);
        const key = `${record.qry_contig_id}-${record.ref_start_pos}-${record.ref_end_pos}-${record.file_index}`;

        cachedTrack.push({ record, startX, endX, width, color, key });
      }

      newCache.push(cachedTrack);
    }

    lastWinStart = winStart;
    lastWinEnd = winEnd;
    lastFltRecs = stacked.flat();
    cachedBars = newCache;

    return newCache;
  }

  function clearCaches() {
    cachedBars = [];
    lastWinStart = -1;
    lastWinEnd = -1;
    lastFltRecs = [];
    lastStackIn = null;
    cachedStacked = [];
  }

  // IntersectionObserver-driven lazy init.
  let observer: IntersectionObserver;

  let unsubSrch: (() => void) | null = null;
  let unsubFlt: (() => void) | null = null;

  function initSubs() {
    if (unsubSrch || unsubFlt) return;

    unsubSrch = searchStore.subscribe(state => {
      if (state.areaQry !== srchQry) {
        srchQry = state.areaQry;
        if (srchQry.trim() && shouldRerun) {
          runSearch(srchQry.trim());
        }
      }
    });

    unsubFlt = areaFltState.subscribe(state => {
      selGens = state.selFiles;
      selChr = state.selChr;
      winSize = state.winSize;
      curWinIdx = state.curWinIdx;
      submittedQry = state.qry || '';

      if (submittedQry.trim()) {
        isSrch = true;
        srchQry = submittedQry;

        const seqId = parseInt(submittedQry.trim());
        if (!isNaN(seqId)) {
          (async () => {
            const key = cacheKey(selGens, selChr);
            let records: WireAreaRecord[] = [];
            const cached = chrRecCache.get(key);
            if (cached) {
              records = cached.records;
            } else if (sessId && isQueryable) {
              try {
                const resp = await fetchChrRecs(sessId, {
                  genomes: selGens,
                  chr: selChr,
                });
                if (resp) {
                  chrRecCache.set(key, resp);
                  records = resp.records;
                }
              } catch (err) {
                console.error('Restored-search records fetch failed:', err);
              }
            }
            const range = getChrRange(records);
            fltWins = findWinsWithSeq(seqId, records, range, winSize);

            if (fltWins.length > 0) {
              if (!fltWins.includes(curWinIdx)) {
                curWinIdx = fltWins[0];
                areaFltState.update(s => ({
                  ...s,
                  curWinIdx: curWinIdx
                }));
              }
            } else {
              if (curWinIdx !== 0) {
                curWinIdx = 0;
                areaFltState.update(s => ({
                  ...s,
                  curWinIdx: 0
                }));
              }
            }
          })();
        }
      } else {
        isSrch = false;
        fltWins = [];
      }
    });
  }

  onMount(() => {
    observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting && !isInit) {
            isVisible = true;
            isInit = true;
            initSubs();
            shouldRerun = true;

            const unsubInit = areaFltState.subscribe((state) => {
              if (state.qry && state.qry.trim()) {
                srchQry = state.qry;
                runSearch(state.qry.trim());
              }
              unsubInit();
            });
          }
        });
      },
      { root: null, rootMargin: '50px', threshold: 0.1 }
    );

    if (containerEl) {
      observer.observe(containerEl);
    }
  });

  function runSearch(query: string) {
    if (!query.trim()) {
      resetSrch();
      return;
    }

    const seqId = parseInt(query.trim());
    if (!isNaN(seqId)) {
      submittedQry = query.trim();
      isSrch = true;

      const range = getChrRange(chrRecs);
      fltWins = findWinsWithSeq(seqId, chrRecs, range, winSize);

      const targetIdx = fltWins.length > 0
        ? (fltWins.includes(curWinIdx) ? curWinIdx : fltWins[0])
        : 0;
      curWinIdx = targetIdx;

      areaFltState.update(state => ({
        ...state,
        qry: submittedQry,
        curWinIdx: targetIdx
      }));

      searchStore.update(state => ({ ...state, areaQry: query }));
      clearCaches();
    }
  }

  function onSearch() {
    runSearch(srchQry.trim());
  }

  function getChrRange(records: any[]) {
    if (records.length === 0) return { min: 0, max: 100000 };
    const min = Math.min(...records.map(r => r.ref_start_pos));
    const max = Math.max(...records.map(r => r.ref_end_pos));
    return { min: Math.floor(min), max: Math.ceil(max) };
  }

  function stackBars(records: any[], winStart: number, winEnd: number) {
    const visible = records.filter(r =>
      r.ref_end_pos >= winStart && r.ref_start_pos <= winEnd
    );

    visible.sort((a, b) => a.ref_start_pos - b.ref_start_pos);

    const stacked: any[][] = [];
    for (const record of visible) {
      let placed = false;
      for (let trackIdx = 0; trackIdx < stacked.length; trackIdx++) {
        const track = stacked[trackIdx];

        let hasOverlap = false;
        for (const existing of track) {
          if (record.ref_start_pos < existing.ref_end_pos &&
              record.ref_end_pos > existing.ref_start_pos) {
            hasOverlap = true;
            break;
          }
        }

        if (!hasOverlap) {
          track.push(record);
          placed = true;
          break;
        }
      }

      if (!placed) {
        stacked.push([record]);
      }
    }

    return stacked;
  }

  function posToX(pos: number, winStart: number, winSize: number): number {
    const rel = pos - winStart;
    const pct = (rel / winSize) * 100;
    return Math.max(0, Math.min(100, pct));
  }

  function clampWinIdx(newGens: number[]): number {
    if (newGens.length === 0) return 0;
    let refLen = winSize;
    for (const gi of newGens) {
      const chrs = chrInfo[gi] ?? [];
      for (const c of chrs) {
        if (c.ref_contig_id === selChr && c.ref_len > refLen) {
          refLen = c.ref_len;
        }
      }
    }
    const newTotWins = Math.ceil(refLen / winSize);
    return Math.min(curWinIdx, Math.max(0, newTotWins - 1));
  }

  function toggleGen(genIdx: number) {
    let newSelGens: number[];

    if (selGens.includes(genIdx)) {
      newSelGens = selGens.filter(i => i !== genIdx);
    } else {
      newSelGens = [...selGens, genIdx].sort((a, b) => a - b);
    }

    const clamped = clampWinIdx(newSelGens);
    selGens = newSelGens;
    curWinIdx = clamped;

    areaFltState.update(state => ({
      ...state,
      selFiles: newSelGens,
      curWinIdx: clamped
    }));

    clearCaches();
    resetSrch(clamped);
  }

  function selectAllGens() {
    const all = files.map((_, idx) => idx);
    const clamped = clampWinIdx(all);
    selGens = all;
    curWinIdx = clamped;

    areaFltState.update(state => ({
      ...state,
      selFiles: all,
      curWinIdx: clamped
    }));

    clearCaches();
    resetSrch(clamped);
  }

  function clearGens() {
    selGens = [];
    curWinIdx = 0;

    areaFltState.update(state => ({
      ...state,
      selFiles: [],
      curWinIdx: 0
    }));

    clearCaches();
    resetSrch();
  }

  function resetSrch(preserved: number = 0) {
    srchQry = '';
    submittedQry = '';
    isSrch = false;
    fltWins = [];
    curWinIdx = preserved;

    searchStore.update(state => ({ ...state, areaQry: '' }));
    areaFltState.update(state => ({
      ...state,
      qry: '',
      curWinIdx: preserved
    }));

    clearCaches();
  }

  function findWinsWithSeq(seqId: number, records: any[], range: any, winSize: number): number[] {
    const seqRecs = records.filter(record => record.qry_contig_id === seqId);
    if (seqRecs.length === 0) return [];

    const wins = new Set<number>();

    for (const record of seqRecs) {
      const startWin = Math.floor((record.ref_start_pos - range.min) / winSize);
      const endWin = Math.floor((record.ref_end_pos - range.min) / winSize);

      for (let w = startWin; w <= endWin; w++) {
        if (w >= 0) {
          wins.add(w);
        }
      }
    }

    return Array.from(wins).sort((a, b) => a - b);
  }

  function onSearchKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      onSearch();
    } else if (e.key === 'Escape') {
      resetSrch();
    }
  }

  function clearSrch() {
    resetSrch();
  }

  function nextWin() {
    if (isSrch && fltWins.length > 0) {
      const inFlt = fltWins.indexOf(curWinIdx);
      if (inFlt < fltWins.length - 1) {
        curWinIdx = fltWins[inFlt + 1];
        areaFltState.update(state => ({ ...state, curWinIdx: curWinIdx }));
      }
    } else {
      curWinIdx++;
      areaFltState.update(state => ({ ...state, curWinIdx: curWinIdx }));
    }
  }

  function prevWin() {
    if (isSrch && fltWins.length > 0) {
      const inFlt = fltWins.indexOf(curWinIdx);
      if (inFlt > 0) {
        curWinIdx = fltWins[inFlt - 1];
        areaFltState.update(state => ({ ...state, curWinIdx: curWinIdx }));
      }
    } else {
      curWinIdx--;
      areaFltState.update(state => ({ ...state, curWinIdx: curWinIdx }));
    }
  }

  $: fltChrRecs = isSrch
    ? chrRecs.filter(record => {
        const seqId = parseInt(submittedQry);
        if (isNaN(seqId)) return false;
        return record.qry_contig_id === seqId;
      })
    : chrRecs;
  $: chrRange = getChrRange(chrRecs);
  $: chrRefLen = chrRefLenFetched > 0
    ? chrRefLenFetched
    : (chrRecs.length > 0 ? chrRecs[0].ref_len : winSize);

  $: totWins = Math.ceil(chrRefLen / winSize);
  $: effTotWins = isSrch ? fltWins.length : totWins;
  $: effCurWinIdx = isSrch ?
    (fltWins.indexOf(curWinIdx) + 1 || 1) :
    (curWinIdx + 1);

  $: winStart = chrRange.min + (curWinIdx * winSize);
  $: winEnd = Math.min(winStart + winSize, chrRefLen);

  /** Memoised stacked tracks for this window. */
  $: stacked = isVisible ? getCachedStacked(fltChrRecs, winStart, winEnd) : [];
  /** Memoised bar geometry. */
  $: bars = isVisible ? genCachedBars(stacked, winStart, winEnd, winSize) : [];
  /** Sorted unique sequence ids — drives the legend list. */
  $: uniqSeqs = Array.from(new Set(fltChrRecs.map(r => r.qry_contig_id))).sort((a, b) => a - b);

  $: canGoPrev = isSrch ?
    fltWins.indexOf(curWinIdx) > 0 :
    curWinIdx > 0;
  $: canGoNext = isSrch ?
    fltWins.indexOf(curWinIdx) < fltWins.length - 1 :
    curWinIdx < totWins - 1;

  const CHRS = Array.from({ length: 24 }, (_, i) => i + 1);

  function onChrChange() {
    curWinIdx = 0;

    areaFltState.update(state => ({
      ...state,
      selChr: selChr,
      curWinIdx: 0
    }));

    clearCaches();

    if (isSrch && submittedQry) {
      runSearch(submittedQry);
    }
  }

  let editWinPage = false;
  let winPageInput = '';

  function startEditWinPage() {
    editWinPage = true;
    winPageInput = effCurWinIdx.toString();
  }

  function submitWinJump() {
    const n = parseInt(winPageInput);
    if (!isNaN(n)) {
      if (isSrch && fltWins.length > 0) {
        const newFltIdx = Math.max(0, Math.min(n - 1, fltWins.length - 1));
        curWinIdx = fltWins[newFltIdx];
      } else {
        const newIdx = Math.max(0, Math.min(n - 1, totWins - 1));
        curWinIdx = newIdx;
      }
      areaFltState.update(state => ({ ...state, curWinIdx: curWinIdx }));
    }
    editWinPage = false;
  }

  function onWinPageKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      submitWinJump();
    } else if (e.key === 'Escape') {
      editWinPage = false;
    }
  }

  /** Flat lookup map from bar key → record. */
  let barKeyMap = new Map<string, any>();
  $: {
    barKeyMap = new Map();
    for (const track of bars) {
      for (const bar of track) {
        barKeyMap.set(bar.key, bar.record);
      }
    }
  }

  /** Delegated mousemove handler. */
  function onBarMouseMove(e: MouseEvent) {
    const target = (e.target as HTMLElement).closest('[data-contig-key]') as HTMLElement | null;
    if (!target) {
      if (hoverRec !== null) hoverRec = null;
      return;
    }
    const key = target.dataset.contigKey!;
    const record = barKeyMap.get(key) ?? null;
    if (hoverRec !== record) hoverRec = record;
  }

  function onBarMouseLeave() {
    hoverRec = null;
  }

  onDestroy(() => {
    if (unsubSrch) unsubSrch();
    if (unsubFlt) unsubFlt();
    if (observer) observer.disconnect();
    clearCaches();
    colorCache.clear();
  });
</script>

<div class="analysis-container" bind:this={containerEl}>
  {#if !isVisible}
    <div class="lazy-placeholder">
      <div class="lazy-spinner"></div>
      <p>Loading Area Analysis...</p>
    </div>
  {:else}
    <div class="controls">
      <div class="control-group full-width">
        <label for="genome-selection">Select Genomes:</label>
        <div class="file-selection" id="genome-selection">
          {#each files as gen, idx}
            <label class="file-checkbox">
              <input
                type="checkbox"
                checked={selGens.includes(idx)}
                on:change={() => toggleGen(idx)}
              />
              <span class="file-checkbox-label">
                <span class="file-color-indicator" style="background: {gen.color}"></span>
                {gen.name}
              </span>
            </label>
          {/each}
        </div>
        <div class="file-selection-actions">
          <button class="action-btn" on:click={selectAllGens}>Select All</button>
          <button class="action-btn" on:click={clearGens}>Clear All</button>
          <span class="selected-count">{selGens.length} of {files.length} selected</span>
        </div>
      </div>

      <div class="control-group">
        <label for="chromosome-select">Select Chromosome:</label>
        <select id="chromosome-select" bind:value={selChr} on:change={onChrChange}>
          {#each CHRS as chr}
            <option value={chr}>Chromosome {chr}</option>
          {/each}
        </select>
      </div>
    </div>

    {#if uniqSeqs.length > 0}
      <div class="legend">
        <div class="legend-header">
          <h3>
            {#if isSrch}
              Showing Sequence {submittedQry} ({fltWins.length} windows)
            {:else}
              Query Sequences ({uniqSeqs.length})
            {/if}
          </h3>
          <div class="search-bar">
            <input
              type="text"
              placeholder="Search sequence ID and press Enter..."
              bind:value={srchQry}
              on:keydown={onSearchKeydown}
              class="search-input"
            />
          </div>
        </div>
        {#if !isSrch}
          <div class="legend-items">
            {#each uniqSeqs as seqId}
              <div class="legend-item">
                <div class="legend-color" style="background: {seqColor(seqId)}"></div>
                <span>QryContig {seqId}</span>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {:else if selGens.length > 0}
      <div class="legend">
        <div class="legend-header">
          <h3>No Sequences Found</h3>
          <div class="search-bar">
            <input
              type="text"
              placeholder="Search sequence ID and press Enter..."
              bind:value={srchQry}
              on:keydown={onSearchKeydown}
              class="search-input"
            />
          </div>
        </div>
      </div>
    {/if}

    <div class="overview-panel" class:open={ovPanelOpen}>
      <button
        class="overview-toggle"
        on:click={() => ovPanelOpen = !ovPanelOpen}
        aria-expanded={ovPanelOpen}
      >
        <svg
          class="toggle-chevron"
          class:rotated={ovPanelOpen}
          width="16" height="16" viewBox="0 0 16 16" fill="none"
        >
          <path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        <span class="overview-toggle-title">Chromosome Overview</span>
        {#if isSrch && submittedQry}
          <span class="overview-search-badge">Sequence {submittedQry}</span>
        {/if}
      </button>

      {#if ovPanelOpen}
        <div class="overview-body">
          {#if ovData.length === 0}
            <p class="overview-empty">No chromosome data available.</p>
          {:else}
            {#each ovData as gen, gi}
              <div class="overview-genome">
                <div class="overview-genome-header">
                  <span class="overview-genome-dot" style="background: {gen.genColor}"></span>
                  <span class="overview-genome-name">{gen.genName}</span>
                </div>
                <div class="overview-lines">
                  {#each gen.lines as line}
                    <div class="overview-chr-row">
                      <span class="overview-chr-label">Chr {line.chrId}</span>
                      <div class="overview-chr-track">
                        <div class="overview-line-bg"></div>
                        {#each line.markers as markerBp, mi}
                          {@const pct = line.refLen > 0 ? (markerBp / line.refLen) * 100 : 0}
                          <div
                            class="overview-marker"
                            class:overview-marker-end={mi === 0 || mi === line.markers.length - 1}
                            style="left: {pct}%"
                            title="{markerBp.toLocaleString()} bp"
                          >
                            <span class="overview-marker-label" class:overview-marker-label-end={mi === 0 || mi === line.markers.length - 1}>
                              {#if markerBp >= 1e6}
                                {(markerBp / 1e6).toFixed(0)}M
                              {:else if markerBp >= 1e3}
                                {(markerBp / 1e3).toFixed(0)}k
                              {:else}
                                {markerBp}
                              {/if}
                            </span>
                          </div>
                        {/each}
                        {#each line.dots as dot}
                          {@const winBpStart = dot.estWin * winSize}
                          {@const winBpEnd = Math.min(line.refLen, winBpStart + winSize)}
                          {@const bpLabel = `${(winBpStart / 1e3).toFixed(0)}–${(winBpEnd / 1e3).toFixed(0)} kb`}
                          <button
                            class="overview-dot"
                            style="left: {dot.xFrac * 100}%"
                            title="Window {dot.estWin + 1} ({bpLabel})"
                            on:click={() => navFromDot(line.chrId, dot.estWin)}
                          >
                            <span class="overview-dot-tooltip">
                              Window {dot.estWin + 1}<br/>
                              {bpLabel}
                            </span>
                          </button>
                        {/each}
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/each}
          {/if}
        </div>
      {/if}
    </div>

    <div class="comparison-panel" class:open={compPanelOpen}>
      <button
        class="overview-toggle"
        on:click={() => compPanelOpen = !compPanelOpen}
        aria-expanded={compPanelOpen}
      >
        <svg
          class="toggle-chevron"
          class:rotated={compPanelOpen}
          width="16" height="16" viewBox="0 0 16 16" fill="none"
        >
          <path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        <span class="overview-toggle-title">Window Sequence Comparison</span>
        {#if winComp.totUniq > 0}
          <span class="comparison-count-badge">
            {winComp.shared.length} shared · {winComp.totUniq} total
          </span>
        {/if}
      </button>

      {#if compPanelOpen}
        <div class="comparison-body">
          {#if selGens.length < 2}
            <p class="overview-empty">Select at least 2 genomes to compare sequences.</p>
          {:else if winComp.totUniq === 0}
            <p class="overview-empty">No sequences found in this window.</p>
          {:else}
            <div class="comparison-section">
              <div class="comparison-section-header">
                <span class="comparison-section-title">Shared across all genomes</span>
                <span class="comparison-section-count">{winComp.shared.length}</span>
              </div>
              {#if winComp.shared.length > 0}
                <div class="legend-items">
                  {#each winComp.shared as seqId}
                    <div class="legend-item">
                      <div class="legend-color" style="background: {seqColor(seqId)}"></div>
                      <span>QryContig {seqId}</span>
                    </div>
                  {/each}
                </div>
              {:else}
                <p class="comparison-none">No shared sequences in this window.</p>
              {/if}
            </div>

            {#each winComp.uniqPerGen as group}
              <div class="comparison-section">
                <div class="comparison-section-header">
                  <span class="comparison-genome-dot" style="background: {group.genColor}"></span>
                  <span class="comparison-section-title">Only in {group.genName}</span>
                  <span class="comparison-section-count">{group.seqIds.length}</span>
                </div>
                {#if group.seqIds.length > 0}
                  <div class="legend-items">
                    {#each group.seqIds as seqId}
                      <div class="legend-item">
                        <div class="legend-color" style="background: {seqColor(seqId)}"></div>
                        <span>QryContig {seqId}</span>
                      </div>
                    {/each}
                  </div>
                {:else}
                  <p class="comparison-none">No unique sequences.</p>
                {/if}
              </div>
            {/each}
          {/if}
        </div>
      {/if}
    </div>

    <div class="window-info">
      <div class="window-position">
        <strong>Window:</strong> {winStart.toLocaleString()} - {winEnd.toLocaleString()} bp
        {#if isSrch}
          <span class="search-indicator">(Searching: Sequence {submittedQry})</span>
        {/if}
        {#if editWinPage}
          <input
            type="text"
            class="window-page-input"
            bind:value={winPageInput}
            on:keydown={onWinPageKeydown}
            on:blur={submitWinJump}
            on:focus
          />
        {:else}
          <span
            class="window-count"
            on:dblclick={startEditWinPage}
            role="button"
            tabindex="0"
            on:keydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                startEditWinPage();
              }
            }}
          >
            ({effCurWinIdx} / {effTotWins})
            {#if isSrch && fltWins.length > 0}
              <span class="filtered-pages">(Filtered)</span>
            {/if}
          </span>
        {/if}
      </div>
      <div class="window-navigation">
        <button on:click={prevWin} disabled={!canGoPrev}>
          ← Previous
        </button>
        <button on:click={nextWin} disabled={!canGoNext}>
          Next →
        </button>
      </div>
    </div>

    <div class="browser">
      {#if selGens.length === 0}
        <div class="empty-state">
          No genomes selected. Please select one or more genomes to view mappings.
        </div>
      {:else if chrRecs.length === 0}
        <div class="empty-state">
          No mappings found for this chromosome in selected genomes.
        </div>
      {:else if bars.length === 0}
        <div class="empty-state">
          {#if isSrch}
            No occurrences of Sequence {submittedQry} in this window
          {:else}
            No mappings in this window. Use navigation buttons to explore other regions.
          {/if}
        </div>
      {:else}
         <div
          class="browser-inner"
          on:mousemove={onBarMouseMove}
          on:mouseleave={onBarMouseLeave}
          role="presentation"
        >
        <div class="position-markers">
          {#each [0, 0.25, 0.5, 0.75, 1] as fraction}
            {@const pos = winStart + (winSize * fraction)}
            {#if pos <= winEnd}
              <div class="marker" style="left: {fraction * 100}%">
                <div class="marker-tick"></div>
                <div class="marker-label">{Math.round(pos).toLocaleString()}</div>
              </div>
            {/if}
          {/each}
        </div>

        <!-- Scrollable seq area w/ CACHED BARS -->
        <div
          class="contigs-viewport"
          role="presentation"
        >
          <div class="contigs-container">
            {#each bars as track, trackIdx (trackIdx)}
              <div class="contig-track">
                {#each track as bar (bar.key)}
                  <div
                    class="contig"
                    class:hovered={hoverRec === bar.record}
                    style="left: {bar.startX}%; width: {bar.width}%; background: {bar.color}"
                    data-contig-key={bar.key}
                    role="button"
                    tabindex="0"
                  ></div>
                {/each}
              </div>
            {/each}
          </div>
        </div>
        </div>

        <!-- Hover tooltip w/detailed seq info -->
        {#if hoverRec}
          {@const genIdx = fileToGen[hoverRec.file_index] ?? 0}
          <div class="tooltip">
            <div class="tooltip-header">
              Query Sequence {hoverRec.qry_contig_id}
            </div>
            <div class="tooltip-body">
              <div class="tooltip-file">
                <span class="file-badge" style="background: {files[genIdx]?.color}20; color: {files[genIdx]?.color}; border-color: {files[genIdx]?.color}">
                  {files[genIdx]?.name}
                </span>
              </div>
              <div class="tooltip-content">
                <div><strong>Ref Position:</strong> {hoverRec.ref_start_pos.toLocaleString()} - {hoverRec.ref_end_pos.toLocaleString()} bp</div>
                <div><strong>Query Position:</strong> {hoverRec.qry_start_pos.toLocaleString()} - {hoverRec.qry_end_pos.toLocaleString()} bp</div>
                <div><strong>Orientation:</strong> {hoverRec.orientation}</div>
                <div><strong>Confidence:</strong> {hoverRec.confidence.toFixed(2)}</div>
                <div><strong>Ref Length:</strong> {(hoverRec.ref_end_pos - hoverRec.ref_start_pos).toLocaleString()} bp</div>
                <div><strong>Query Length:</strong> {(hoverRec.qry_end_pos - hoverRec.qry_start_pos).toLocaleString()} bp</div>
              </div>
            </div>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .analysis-container {
    padding: 2rem;
    max-width: 1400px;
    margin: 0 auto;
    min-height: 400px;
  }

  .lazy-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1rem;
    padding: 4rem;
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
    min-height: 400px;
  }

  .lazy-spinner {
    width: 40px;
    height: 40px;
    border: 4px solid var(--border-color);
    border-top-color: var(--accent-primary);
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .lazy-placeholder p {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .controls {
    display: grid;
    grid-template-columns: 1fr;
    gap: 1rem;
    margin-bottom: 2rem;
    padding: 1.5rem;
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
  }
  .control-group { display: flex; flex-direction: column; gap: 0.5rem; }
  .control-group.full-width { grid-column: 1 / -1;
  }
  .control-group label {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--text-primary);
  }
  .control-group select {
    padding: 0.5rem;
    border-radius: 0.375rem;
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 0.875rem;
  }

  .file-selection {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin-top: 0.5rem;
  }

  .file-checkbox {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    cursor: pointer;
    padding: 0.5rem 0.75rem;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 0.375rem;
  }
  .file-checkbox input[type="checkbox"] { cursor: pointer; }

  .file-checkbox-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    color: var(--text-primary);
  }

  .file-color-indicator {
    width: 1rem;
    height: 1rem;
    border-radius: 0.25rem;
  }

  .file-selection-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .action-btn {
    padding: 0.375rem 0.75rem;
    background: var(--accent-primary);
    color: white;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.8rem;
    font-weight: 500;
    cursor: pointer;
  }

  .selected-count {
    margin-left: auto;
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  .legend {
    margin-bottom: 2rem;
    padding: 1rem;
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
  }

  .legend-header {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }

  .legend h3 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .legend-items {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    max-height: 10rem;
    overflow-y: auto;
    padding-right: 0.25rem;
    scrollbar-width: thin;
    scrollbar-color: var(--border-color-dark) transparent;
  }

  .legend-items::-webkit-scrollbar { width: 5px; }
  .legend-items::-webkit-scrollbar-track { background: transparent; }
  .legend-items::-webkit-scrollbar-thumb { background: var(--border-color-dark); border-radius: 3px; }
  .legend-item { display: flex; align-items: center; gap: 0.5rem; }
  .legend-color { width: 1.5rem; height: 0.75rem; border-radius: 0.125rem; }
  .legend-item span { font-size: 0.8rem; color: var(--text-secondary); }

  .window-info {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
    padding: 1rem;
    background: var(--bg-accent);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
  }

  .window-position {
    color: var(--text-primary);
    font-size: 0.875rem;
  }

  .window-count {
    margin-left: 1rem;
    color: var(--text-secondary);
    cursor: pointer;
    user-select: none;
    padding: 0.25rem 0.5rem;
    border-radius: 0.25rem;
  }

  .window-page-input {
    width: 5rem;
    padding: 0.25rem 0.5rem;
    margin-left: 0.5rem;
    text-align: center;
    font-size: 0.875rem;
    background: var(--bg-primary);
    border: 2px solid var(--accent-primary);
    border-radius: 0.25rem;
    color: var(--text-primary);
  }

  .search-bar {
    position: relative;
    max-width: 100%;
    width: 100%;
    box-sizing: border-box;
  }

  .search-input {
    width: 100%;
    padding: 0.5rem 2.5rem 0.5rem 0.75rem;
    font-size: 0.8rem;
    border: 1px solid var(--border-color-dark);
    border-radius: 0.375rem;
    background: var(--bg-primary);
    color: var(--text-primary);
    box-sizing: border-box;
  }
  .search-input:focus { outline: none; border-color: var(--accent-primary); }
  .search-input::placeholder { color: var(--text-tertiary); }
  .search-indicator {
    margin-left: 0.5rem;
    font-size: 0.8rem;
    color: var(--accent-primary);
    font-weight: 500;
  }

  .filtered-pages {
    margin-left: 0.25rem;
    font-size: 0.7rem;
    color: var(--accent-primary);
    font-style: italic;
  }

  .window-navigation { display: flex; gap: 0.5rem; }
  .window-navigation button {
    padding: 0.5rem 1rem;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
  }

  .window-navigation button:not(:disabled) {
    background: var(--accent-primary);
    color: white;
  }

  .window-navigation button:disabled {
    background: var(--bg-hover);
    color: var(--text-tertiary);
    cursor: not-allowed;
  }

  .browser {
    position: relative;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
    min-height: 400px;
    display: flex;
    flex-direction: column;
  }
  .browser-inner { position: relative; padding: 0 2rem; }

  .empty-state {
    text-align: center;
    padding: 4rem;
    color: var(--text-secondary);
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .position-markers {
    position: sticky;
    top: 0;
    height: 28px;
    margin-bottom: 2rem;
    background: var(--bg-secondary);
    border-bottom: 2px solid var(--border-color-dark);
    flex-shrink: 0;
    z-index: 20;
    pointer-events: none;
  }

  .marker {
    position: absolute;
    font-size: 0.7rem;
    color: var(--text-secondary);
  }

  .marker-tick {
    width: 1px;
    height: 10px;
    background: var(--border-color-dark);
    margin-top: 1.5rem;
  }

  .marker-label {
    white-space: nowrap;
    margin-left: -1rem;
  }

  .contigs-viewport {
    flex: 1;
    overflow-y: auto;
    padding-bottom: 1rem;
    max-height: 500px;
  }

  .contigs-container {
    display: flex;
    flex-direction: column;
  }

  .contig-track {
    position: relative;
    height: 8px;
    margin-bottom: 0.75rem;
    min-height: 8px;
  }

  .contig {
    position: absolute;
    height: 8px;
    top: 5px;
    cursor: pointer;
    border: 1px solid rgba(0, 0, 0, 0.2);
    border-radius: 3px;
    box-sizing: border-box;
  }

  .contig.hovered {
    border: 2px solid white;
    z-index: 15;
  }

  .tooltip {
    position: fixed;
    bottom: 2rem;
    left: 50%;
    transform: translateX(-50%);
    padding: 1rem;
    background: var(--bg-primary);
    border: 2px solid var(--border-color-dark);
    border-radius: 0.5rem;
    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
    z-index: 1000;
    min-width: 320px;
    max-width: 400px;
  }

  .tooltip-header {
    margin-bottom: 0.5rem;
    font-weight: 600;
    color: var(--accent-primary);
    font-size: 0.95rem;
  }

  .tooltip-body {
    display: flex;
    gap: 0.5rem;
    flex-direction: column;
  }

  .tooltip-file {
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--border-color);
  }

  .file-badge {
    padding: 0.25rem 0.5rem;
    border: 1px solid;
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    white-space: nowrap;
  }

  .tooltip-content {
    font-size: 0.8rem;
    color: var(--text-secondary);
    display: grid;
    gap: 0.25rem;
  }

  .tooltip-content strong {
    color: var(--text-primary);
  }

  .overview-panel {
    margin-bottom: 1rem;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
    overflow: hidden;
  }

  .overview-toggle {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    width: 100%;
    padding: 0.75rem 1rem;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-primary);
    font-size: 0.875rem;
    font-weight: 600;
    text-align: left;
  }
  .overview-toggle:hover { background: var(--bg-hover); }

  .toggle-chevron {
    flex-shrink: 0;
    transition: transform 0.2s ease;
    color: var(--text-secondary);
  }

  .toggle-chevron.rotated { transform: rotate(90deg); }
  .overview-toggle-title { flex: 1; }

  .overview-search-badge {
    padding: 0.15rem 0.5rem;
    background: var(--accent-light);
    color: var(--accent-primary);
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
  }

  .overview-body {
    padding: 0 1rem 1rem;
    padding-top: 0.5rem;
    padding-bottom: 1.5rem;
    display: flex;
    gap: 1.25rem;
    flex-direction: column;
    animation: fadeIn 0.15s ease-in;
  }

  .overview-empty {
    color: var(--text-secondary);
    font-size: 0.8rem;
    text-align: center;
    padding: 1rem 0;
  }

  .overview-genome {
    display: flex;
    gap: 0.25rem;
    flex-direction: column;
  }

  .overview-genome-header {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    margin-bottom: 0.25rem;
  }

  .overview-genome-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .overview-genome-name {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .overview-lines {
    display: flex;
    gap: 4px;
    flex-direction: column;
  }

  .overview-chr-row {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    height: 32px;
  }

  .overview-chr-label {
    width: 42px;
    flex-shrink: 0;
    font-size: 0.65rem;
    color: var(--text-tertiary);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .overview-chr-track {
    position: relative;
    flex: 1;
    height: 18px;
    display: flex;
    align-items: center;
    margin-right: 2rem;
  }

  .overview-line-bg {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 2px;
    background: var(--border-color);
    transform: translateY(-50%);
  }

  .overview-marker {
    position: absolute;
    top: 50%;
    width: 1px;
    height: 8px;
    background: var(--border-color);
    transform: translate(-50%, -50%);
  }

  .overview-marker-end {
    height: 12px;
    background: var(--text-secondary);
  }

  .overview-marker-label {
    position: absolute;
    top: 9px;
    left: 50%;
    transform: translateX(-50%);
    font-size: 0.65rem;
    opacity: 0.85;
    color: var(--text-secondary);
    white-space: nowrap;
    pointer-events: none;
  }

  .overview-marker-label.overview-marker-label-end {
    font-size: 0.7rem;
    opacity: 1;
    font-weight: 600;
  }

  .overview-dot {
    position: absolute;
    top: 50%;
    width: 8px;
    height: 8px;
    /* Green = "search hit"; distinct from accent-coloured chromosome markers. */
    background: #22c55e;
    border: 1px solid var(--bg-secondary);
    border-radius: 50%;
    transform: translate(-50%, -50%);
    cursor: pointer;
    padding: 0;
    z-index: 2;
  }

  .overview-dot-tooltip {
    display: none;
    position: absolute;
    bottom: 14px;
    left: 50%;
    transform: translateX(-50%);
    padding: 0.3rem 0.5rem;
    background: var(--bg-primary);
    color: var(--text-primary);
    border: 1px solid var(--border-color-dark);
    border-radius: 0.25rem;
    font-size: 0.7rem;
    line-height: 1.4;
    white-space: nowrap;
    pointer-events: none;
    text-align: center;
  }

  .overview-dot:hover .overview-dot-tooltip {
    display: block;
  }

  .comparison-panel {
    margin-bottom: 1rem;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
    overflow: hidden;
  }

  .comparison-count-badge {
    padding: 0.15rem 0.5rem;
    background: var(--bg-hover);
    color: var(--text-secondary);
    border-radius: 0.25rem;
    font-size: 0.7rem;
    font-weight: 500;
  }

  .comparison-body {
    padding: 0 1rem 0.5rem;
    max-height: 20rem;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--border-color-dark) transparent;
    animation: fadeIn 0.15s ease-in;
  }
  .comparison-body::-webkit-scrollbar { width: 5px; }
  .comparison-body::-webkit-scrollbar-track { background: transparent;}
  .comparison-body::-webkit-scrollbar-thumb { background: var(--border-color-dark); border-radius: 3px; }
  .comparison-section { padding: 0.5rem 0; }
  .comparison-section + .comparison-section {
    border-top: 1px solid var(--border-color);
    padding-top: 0.75rem;
  }
  .comparison-section-header {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    margin-bottom: 0.4rem;
  }
  .comparison-genome-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .comparison-section-title {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-primary);
    flex: 1;
  }

  .comparison-section-count {
    font-size: 0.7rem;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .comparison-none {
    font-size: 0.75rem;
    color: var(--text-tertiary);
    font-style: italic;
    margin: 0;
  }

  @media (max-width: 768px) {
    .analysis-container {
      padding: 1rem;
    }

    .window-info {
      flex-direction: column;
      gap: 1rem;
    }

    .file-selection {
      flex-direction: column;
    }

    .position-markers { padding: 0 1rem; }

    .contigs-viewport { padding: 1rem; }

    .tooltip {
      left: 1rem;
      right: 1rem;
      transform: none;
      max-width: calc(100% - 2rem);
    }
  }

  .action-btn:hover {
    filter: brightness(0.88);
    transition: filter 0.15s ease;
  }

  .window-navigation button:not(:disabled):hover {
    filter: brightness(0.88);
    transition: filter 0.15s ease;
  }
</style>