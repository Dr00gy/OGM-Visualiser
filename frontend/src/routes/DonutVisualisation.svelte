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
  export let fileToGen: number[] = [];
  export let matches: BackendMatch[] = [];
  export let chrInfo: ChromosomeInfo[][] = [];

  export let showDups = false;
  export let isStreaming = false;
  export let sessId: string | null = null;
  export let isQueryable: boolean = false;
</script>

<D3DonutChart
  {files}
  {fileToGen}
  {matches}
  {chrInfo}
  {showDups}
  {isStreaming}
  {sessId}
  {isQueryable}
/>