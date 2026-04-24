<script lang="ts" context="module">
  export interface FileData {
    name: string;
    rows: number;
    color: string;
  }
</script>

<script lang="ts">
  import type { BackendMatch, ChromosomeInfo } from '$lib/bincodeDecoder';
  import D3DonutChart from './D3DonutChart.svelte';

  export let files: FileData[] = [];
  export let fileToGenome: number[] = [];
  export let matches: BackendMatch[] = [];
  export let chromosomeInfo: ChromosomeInfo[][] = [];

  export let showDuplicates = false;
  export let isStreaming = false;
  export let sessionId: string | null = null;
  export let isQueryable: boolean = false;
</script>

<D3DonutChart
  {files}
  {fileToGenome}
  {matches}
  {chromosomeInfo}
  {showDuplicates}
  {isStreaming}
  {sessionId}
  {isQueryable}
/>