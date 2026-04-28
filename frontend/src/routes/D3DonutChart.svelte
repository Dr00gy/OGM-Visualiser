<script lang="ts">
  import { onMount, afterUpdate, onDestroy } from 'svelte';
  import * as d3 from 'd3';
  import type { ChromosomeInfo } from '$lib/bincodeDecoder';
  import type { FileData } from '$lib/types';
  import { donutFltState } from '$lib/filterStateStore';
  import {
    fetchMeta,
    fetchFlows,
    makeDebouncer,
    type WireFlow,
  } from '$lib/queryClient';
  import DonutInfo from './DonutInfo.svelte';

  export let files: FileData[] = [];
  export let fileToGen: number[] = [];
  export let chrInfo: ChromosomeInfo[][] = [];
  export let showDups = false;
  export let scale = 1.1;
  export let isStreaming = false;
  export let sessId: string | null = null;
  export let isQueryable: boolean = false;

  let svgEl: SVGSVGElement;
  let containerEl: HTMLDivElement;

  let curZoom = 1;
  let curTX = 0;
  let curTY = 0;

  let isInit = false;
  let selSeqId = $donutFltState.selSeqId;
  let selGen1 = $donutFltState.selGen1;
  let selGen2 = $donutFltState.selGen2;
  let selChr = $donutFltState.selChr;
  let selGenForChr = $donutFltState.selGenForChr;

  $: donutFltState.set({
    selSeqId, selGen1, selGen2, selChr, selGenForChr, showDups, scale,
  });

  $: genSizes = (() => {
    const sizes = new Map<number, number>();
    chrInfo.forEach((chroms, genIdx) => {
      const seen = new Map<number, number>();
      for (const c of chroms) {
        if (!seen.has(c.ref_contig_id)) seen.set(c.ref_contig_id, c.ref_len);
      }
      const total = Array.from(seen.values()).reduce((s, v) => s + v, 0);
      sizes.set(genIdx, total);
    });
    files.forEach((_, idx) => {
      if (!sizes.has(idx) || sizes.get(idx) === 0) sizes.set(idx, 100000);
    });
    return sizes;
  })();

  $: totGenSize = Array.from(genSizes.values()).reduce((s, v) => s + v, 0);

  let maxConf: number = 1.0;
  let availSeqIds: number[] = [];
  let metaFor: string | null = null;

  async function loadMeta() {
    if (!sessId || !isQueryable) return;
    if (metaFor === sessId) return;
    metaFor = sessId;
    try {
      const meta = await fetchMeta(sessId);
      if (!meta) return;
      maxConf = meta.max_confidence > 0 ? meta.max_confidence : 1.0;
      availSeqIds = meta.available_sequence_ids;
    } catch (err) {
      console.error('Failed to fetch /meta:', err);
      metaFor = null;
    }
  }

  $: if (sessId && isQueryable) {
    loadMeta();
  } else if (!sessId) {
    maxConf = 1.0;
    availSeqIds = [];
    metaFor = null;
  }

  $: availGens = files.map((f, i) => ({ value: i.toString(), label: f.name, color: f.color }));
  $: availChrs = Array.from({ length: 24 }, (_, i) => (i + 1).toString());

  $: cx = 200;
  $: cy = 200;
  $: baseR = 80;
  $: baseSW = 20;
  $: radius = baseR * scale;
  $: sw = baseSW * scale;
  $: circleR = radius - sw / 2;
  $: circumference = 2 * Math.PI * circleR;

  $: segments = (() => {
    if (totGenSize === 0) return [];
    let offset = 0;
    return files.map((file, idx) => {
      const genSize = genSizes.get(idx) || 1;
      const pct = genSize / totGenSize;
      const length = pct * circumference;
      const startAng = (offset / circumference) * 360 - 90;
      const endAng = ((offset + length) / circumference) * 360 - 90;
      const angRange = endAng - startAng;
      const dashArray = `${length} ${circumference}`;
      const dashOffset = -offset;
      offset += length;
      return {
        ...file, idx, genSize,
        pct: (pct * 100).toFixed(1),
        showLabel: pct >= 0.01,
        showChrs: pct >= 0.20,
        startAng, endAng, angRange,
        dashArray, dashOffset,
      };
    });
  })();

  $: parsedFlts = {
    qry:   selSeqId !== ''      ? parseInt(selSeqId)      : null,
    g1:    selGen1 !== ''       ? parseInt(selGen1)       : null,
    g2:    selGen2 !== ''       ? parseInt(selGen2)       : null,
    chr:   selChr !== ''        ? parseInt(selChr)        : null,
    chrG:  selGenForChr !== ''  ? parseInt(selGenForChr)  : null,
  };

  let flowPaths: any[] = [];
  let flowsLdg: boolean = false;
  let flowsAbort: AbortController | null = null;
  const flowsDeb = makeDebouncer(400);

  function enrichFlow(wf: WireFlow): any {
    const avgConf = (wf.from_confidence + wf.to_confidence) / 2;
    const norm = avgConf / maxConf;
    return {
      fromFileIdx: wf.from_genome,
      fromChr: wf.from_chromosome,
      toFileIdx: wf.to_genome,
      toChr: wf.to_chromosome,
      color: files[wf.from_genome]?.color || '#888',
      opacity: 0.1 + (norm * 0.9),
      width: (1 + norm * 2) * scale,
      conf: Math.max(wf.from_confidence, wf.to_confidence),
      isSameGen: wf.from_genome === wf.to_genome,
      qryContigId: wf.qry_contig_id,
      fromRec: {
        file_index: -1,
        ref_contig_id: wf.from_chromosome,
        orientation: wf.from_orientation,
        confidence: wf.from_confidence,
      },
      toRec: {
        file_index: -1,
        ref_contig_id: wf.to_chromosome,
        orientation: wf.to_orientation,
        confidence: wf.to_confidence,
      },
    };
  }

  const FLOW_RENDER_LIMIT = 5000;
  $: anyFltActive =
    selSeqId !== '' || selGen1 !== '' || selGen2 !== '' || selChr !== '';

  async function reloadFlows() {
    if (!sessId || !isQueryable) {
      flowPaths = [];
      return;
    }

    if (!anyFltActive) {
      if (flowsAbort) {
        flowsAbort.abort();
        flowsAbort = null;
      }
      flowPaths = [];
      flowsLdg = false;
      return;
    }

    if (flowsAbort) flowsAbort.abort();
    flowsAbort = new AbortController();
    const signal = flowsAbort.signal;

    const chipTimer = setTimeout(() => { flowsLdg = true; }, 200);

    try {
      const p = parsedFlts;
      const wireFlows = await fetchFlows(sessId, {
        qry: p.qry ?? undefined,
        g1:  p.g1  ?? undefined,
        g2:  p.g2  ?? undefined,
        chr: p.chr ?? undefined,
        chrGen: p.chrG ?? undefined,
        showDups,
        limit: FLOW_RENDER_LIMIT,
        signal,
      });
      if (wireFlows === undefined) return;
      flowPaths = wireFlows.map(enrichFlow);
    } catch (err) {
      console.error('Failed to fetch flows:', err);
      flowPaths = [];
    } finally {
      clearTimeout(chipTimer);
      flowsLdg = false;
    }
  }

  $: if (isQueryable && sessId) {
    selSeqId; selGen1; selGen2; selChr; selGenForChr; showDups;
    flowsDeb.schedule(() => reloadFlows());
  }

  $: if (isInit && mainGroup) {
    flowPaths;
    updateChart();
  }

  function clearAllFlts() {
    selSeqId = ''; selGen1 = ''; selGen2 = ''; selChr = ''; selGenForChr = '';
  }

  $: showChrLabels = scale >= 1.1;
  $: chrNodes = (() => {
    if (!files.length) return [];

    const nodes: Array<{
      id: string; fileIdx: number; chr: number;
      angle: number; x: number; y: number; color: string;
    }> = [];

    for (let fileIdx = 0; fileIdx < files.length; fileIdx++) {
      const seg = segments[fileIdx];
      if (!seg) continue;

      const segStart = seg.startAng;
      const segRange = seg.angRange;

      for (let i = 1; i <= 24; i++) {
        const chrMidDeg = segStart + (segRange * (i - 0.5) / 24);
        const rad = (chrMidDeg * Math.PI) / 180;
        const x = cx + (radius - sw) * Math.cos(rad);
        const y = cy + (radius - sw) * Math.sin(rad);
        nodes.push({
          id: `chr_${fileIdx}_${i}`,
          fileIdx, chr: i,
          angle: chrMidDeg,
          x, y,
          color: files[fileIdx].color,
        });
      }
    }
    return nodes;
  })();

  let mainGroup: d3.Selection<SVGGElement, unknown, null, undefined>;
  let flowsLayer: d3.Selection<SVGGElement, unknown, null, undefined>;
  let donutLayer: d3.Selection<SVGGElement, unknown, null, undefined>;
  let ticksLayer: d3.Selection<SVGGElement, unknown, null, undefined>;
  let labelsLayer: d3.Selection<SVGGElement, unknown, null, undefined>;

  function initChart() {
    if (!svgEl || !files.length) return;

    d3.select(svgEl).selectAll('*').remove();

    const svg = d3
      .select(svgEl)
      .attr('width', 400)
      .attr('height', 400)
      .attr('viewBox', '0 0 400 400')
      .attr('preserveAspectRatio', 'xMidYMid meet');

    mainGroup = svg.append('g').attr('class', 'main-group');
    flowsLayer = mainGroup.append('g').attr('class', 'flow-lines');
    donutLayer = mainGroup.append('g').attr('class', 'donut-segments');
    ticksLayer = mainGroup.append('g').attr('class', 'chromosome-markers');
    labelsLayer = mainGroup.append('g').attr('class', 'chromosome-labels');

    mainGroup.attr('transform', `translate(${curTX},${curTY}) scale(${curZoom})`);

    const zoom = d3.zoom()
      .scaleExtent([0.5, 5])
      .on('zoom', (event) => {
        mainGroup.attr('transform', event.transform);
        curZoom = event.transform.k;
        curTX = event.transform.x;
        curTY = event.transform.y;
      });

    svg.call(zoom as any);
    if (curZoom !== 1 || curTX !== 0 || curTY !== 0) {
      svg.call(zoom.transform as any, d3.zoomIdentity.translate(curTX, curTY).scale(curZoom));
    }

    isInit = true;
    updateChart();
  }

  function updateChart() {
    if (!mainGroup || !flowsLayer || !ticksLayer || !labelsLayer || !donutLayer) return;

    const cumOffsets = new Map<number, number>();
    let off = 0;
    segments.forEach((s) => {
      cumOffsets.set(s.idx, off);
      off += (s.angRange / 360) * circumference;
    });

    donutLayer
      .selectAll('circle.segment')
      .data(segments, (d: any) => d.idx)
      .join(
        enter => enter
          .append('circle')
          .attr('class', 'segment')
          .attr('cx', cx)
          .attr('cy', cy)
          .attr('r', circleR)
          .attr('fill', 'transparent')
          .attr('stroke', (d: any) => d.color)
          .attr('stroke-width', sw)
          .attr('stroke-dasharray', (d: any) => `${(d.angRange / 360) * circumference} ${circumference}`)
          .attr('stroke-dashoffset', (d: any) => -(cumOffsets.get(d.idx) || 0))
          .attr('transform', `rotate(-90 ${cx} ${cy})`),
        update => update
          .attr('r', circleR)
          .attr('stroke-width', sw)
          .attr('stroke', (d: any) => d.color)
          .attr('stroke-dasharray', (d: any) => `${(d.angRange / 360) * circumference} ${circumference}`)
          .attr('stroke-dashoffset', (d: any) => -(cumOffsets.get(d.idx) || 0)),
        exit => exit.remove(),
      );

    const ticks: Array<{ x1: number; y1: number; x2: number; y2: number; key: string }> = [];
    for (const seg of segments) {
      const start = seg.startAng * Math.PI / 180;
      const end = seg.endAng * Math.PI / 180;
      const range = end - start;

      for (let i = 0; i <= 24; i++) {
        const a = start + (range * i / 24);
        const x1 = cx + (radius - sw) * Math.cos(a);
        const y1 = cy + (radius - sw) * Math.sin(a);
        const x2 = cx + radius * Math.cos(a);
        const y2 = cy + radius * Math.sin(a);
        ticks.push({ x1, y1, x2, y2, key: `t-${seg.idx}-${i}` });
      }
    }

    ticksLayer
      .selectAll('line.tick')
      .data(ticks, (d: any) => d.key)
      .join(
        enter => enter
          .append('line')
          .attr('class', 'tick')
          .attr('x1', d => d.x1).attr('y1', d => d.y1)
          .attr('x2', d => d.x2).attr('y2', d => d.y2)
          .attr('stroke', 'var(--text-primary)')
          .attr('stroke-width', 1 * scale)
          .attr('opacity', 0.7),
        update => update
          .attr('x1', d => d.x1).attr('y1', d => d.y1)
          .attr('x2', d => d.x2).attr('y2', d => d.y2)
          .attr('stroke-width', 1 * scale),
        exit => exit.remove(),
      );

    const flowsToRender = isStreaming ? flowPaths.slice(0, 500) : flowPaths;

    const flowLines = flowsToRender.map(flow => {
      const fromNode = chrNodes.find(n => n.fileIdx === flow.fromFileIdx && n.chr === flow.fromChr);
      const toNode = chrNodes.find(n => n.fileIdx === flow.toFileIdx && n.chr === flow.toChr);
      return (fromNode && toNode) ? { ...flow, fromNode, toNode } : null;
    }).filter(Boolean) as any[];

    flowsLayer
      .selectAll('path.flow')
      .data(flowLines, (d: any) => `${d.qryContigId}-${d.fromFileIdx}-${d.fromChr}-${d.toFileIdx}-${d.toChr}`)
      .join(
        enter => enter
          .append('path')
          .attr('class', 'flow')
          .attr('d', (d: any) => `M ${d.fromNode.x} ${d.fromNode.y} Q ${cx} ${cy} ${d.toNode.x} ${d.toNode.y}`)
          .attr('stroke', (d: any) => d.color)
          .attr('stroke-width', (d: any) => d.width)
          .attr('fill', 'none')
          .attr('opacity', (d: any) => d.opacity)
          .attr('stroke-linecap', 'round'),
        update => update
          .attr('d', (d: any) => `M ${d.fromNode.x} ${d.fromNode.y} Q ${cx} ${cy} ${d.toNode.x} ${d.toNode.y}`)
          .attr('stroke-width', (d: any) => d.width)
          .attr('opacity', (d: any) => d.opacity),
        exit => exit.remove(),
      );

    const chromLabels = labelsLayer
      .selectAll('text.chrom-label')
      .data(showChrLabels ? chrNodes.filter(d => d.chr % 2 === 1) : [], (d: any) => d.id);

    chromLabels.join(
      enter => enter
        .append('text')
        .attr('class', 'chrom-label')
        .attr('x', d => cx + (radius + 10 * scale) * Math.cos((d.angle * Math.PI) / 180))
        .attr('y', d => cy + (radius + 10 * scale) * Math.sin((d.angle * Math.PI) / 180))
        .attr('text-anchor', 'middle')
        .attr('dominant-baseline', 'middle')
        .attr('font-size', 7 * scale)
        .attr('font-weight', 600)
        .attr('fill', 'var(--text-primary)')
        .attr('opacity', 0.9)
        .text(d => d.chr),
      update => update
        .attr('x', d => cx + (radius + 10 * scale) * Math.cos((d.angle * Math.PI) / 180))
        .attr('y', d => cy + (radius + 10 * scale) * Math.sin((d.angle * Math.PI) / 180))
        .attr('font-size', 7 * scale),
      exit => exit.remove(),
    );

    const centerDot = mainGroup.selectAll('circle.center').data([1]);
    centerDot.join(
      enter => enter
        .append('circle')
        .attr('class', 'center')
        .attr('cx', cx).attr('cy', cy).attr('r', 2)
        .attr('fill', 'var(--text-secondary)'),
      update => update.attr('cx', cx).attr('cy', cy),
      exit => exit.remove(),
    );
  }

  function resetZoom() {
    if (!svgEl) return;
    const svg = d3.select(svgEl);
    svg.call(d3.zoom().transform as any, d3.zoomIdentity);
    curZoom = 1; curTX = 0; curTY = 0;
    if (mainGroup) mainGroup.attr('transform', `translate(0,0) scale(1)`);
  }

  afterUpdate(() => {
    if (!isInit && chrInfo.length > 0 && files.length > 0) initChart();
  });

  onMount(() => {
    if (chrInfo.length > 0 && files.length > 0) initChart();
  });

  onDestroy(() => {
    if (flowsAbort) {
      flowsAbort.abort();
      flowsAbort = null;
    }
    flowsDeb.cancel();
  });
</script>

<div class="container">
  <div class="chart-section">
    <div class="controls">
      <div class="stats">
        <span>{files.length} genomes</span>
        <span>Total genome size: {totGenSize.toLocaleString()} bp</span>
        <span>Flow lines: {flowPaths.length.toLocaleString()}{flowsLdg ? ' (loading...)' : ''} {showDups ? '(self-flow)' : '(cross-genome)'}</span>
        <span class="confidence-stat">Max confidence: {maxConf.toFixed(2)}</span>
      </div>
    </div>

    {#if !files.length || !chrInfo.length}
      <div class="no-data">
        {#if !files.length}
          No data to display. Upload XMAP files to begin.
        {:else}
          <div class="loading-state">
            <div class="spinner"></div>
            <span>Initialising visualization...</span>
          </div>
        {/if}
      </div>
    {:else}
      <div class="chart-wrapper">
        <div class="zoom-controls">
          <button class="zoom-btn" on:click={resetZoom} title="Reset zoom" aria-label="Reset zoom">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
              <path d="M2 8a6 6 0 0 1 10.5-4M14 8a6 6 0 0 1-10.5 4" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
              <path d="M12.5 2v4h-4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </button>
          <span class="zoom-indicator">{(curZoom * 100).toFixed(0)}%</span>
        </div>
        <div class="chart-container" bind:this={containerEl}>
          <svg bind:this={svgEl} class="chart-svg"></svg>
        </div>
        <div class="zoom-hint">
          Scroll to zoom • Drag to pan
        </div>
        {#if isStreaming}
          <div class="streaming-notice">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
              <circle cx="8" cy="8" r="3" fill="currentColor" opacity="0.3">
                <animate attributeName="r" values="3;6;3" dur="1.5s" repeatCount="indefinite"/>
                <animate attributeName="opacity" values="0.3;0;0.3" dur="1.5s" repeatCount="indefinite"/>
              </circle>
              <circle cx="8" cy="8" r="2" fill="currentColor"/>
            </svg>
            Waiting for match phase to finish — flows load once complete.
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <DonutInfo
    {files}
    {fileToGen}
    {segments}
    {genSizes}
    {totGenSize}
    fltFlowPaths={flowPaths}
    {showDups}
    {sessId}
    {isQueryable}
    bind:selSeqId
    bind:selGen1
    bind:selGen2
    bind:selChr
    bind:selGenForChr
    {availSeqIds}
    {availGens}
    {availChrs}
    {clearAllFlts}
  />
</div>

<style>
  .container {
    display: flex;
    gap: 3rem;
    align-items: flex-start;
    flex-wrap: wrap;
  }

  .chart-section {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    max-width: 100%;
  }

  .controls {
    display: flex;
    gap: 0.5rem;
    flex-direction: column;
    margin-top: 0.5rem;
  }

  .stats {
    font-size: 0.8rem;
    color: var(--text-secondary);
    display: grid;
    gap: 0.25rem 0.75rem;
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .confidence-stat {
    font-weight: 600;
    color: var(--accent-primary);
  }

  .chart-wrapper {
    position: relative;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
    padding: 1rem;
    width: clamp(260px, 90vw, 500px);
    box-sizing: border-box;
  }

  .zoom-controls {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin-bottom: 0.5rem;
  }

  .zoom-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 0.375rem;
    color: var(--text-primary);
    cursor: pointer;
  }

  .zoom-btn:hover {
    color: white;
    background: var(--accent-primary);
    border-color: var(--accent-primary);
  }

  .zoom-indicator {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--text-secondary);
    background: var(--bg-primary);
    border-radius: 0.25rem;
    padding: 0.25rem 0.5rem;
  }

  .chart-container {
    width: 100%;
    height: 500px;
    overflow: hidden;
    cursor: grab;
    background: var(--bg-primary);
    border-radius: 0.375rem;
  }
  .chart-container:active { cursor: grabbing; }
  .chart-svg {
    display: block;
    width: 100%;
    height: 100%;
  }

  .zoom-hint {
    margin-top: 0.5rem;
    padding-top: 0.5rem;
    font-size: 0.7rem;
    color: var(--text-tertiary);
    text-align: center;
  }

  .no-data {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4rem;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
    color: var(--text-secondary);
    min-height: 300px;
  }

  .loading-state {
    display: flex;
    gap: 1rem;
    flex-direction: column;
    align-items: center;
  }

  .spinner {
    width: 40px;
    height: 40px;
    border: 4px solid var(--accent-primary);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .streaming-notice {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin-top: 0.75rem;
    padding: 0.75rem;
    background: var(--accent-light);
    border: 1px solid var(--accent-primary);
    border-radius: 0.375rem;
    color: var(--accent-primary);
    font-size: 0.8rem;
    font-weight: 500;
  }

  @media (max-width: 1024px) {
    .stats {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 768px) {
    .container {
      flex-direction: column;
      gap: 1.25rem;
    }
    .chart-container {
      height: 400px;
    }
  }
</style>