<script lang="ts">
  import type { ChromosomeInfo } from '$lib/bincodeDecoder';
  import type { FileData } from '$lib/types';
  import D3DonutChart from './D3DonutChart.svelte';

  export let files: FileData[] = [];
  export let fileToGen: number[] = [];
  export let chrInfo: ChromosomeInfo[][] = [];

  export let showDups = false;
  export let isStreaming = false;
  export let sessId: string | null = null;
  export let isQueryable: boolean = false;
</script>

<D3DonutChart
  {files}
  {fileToGen}
  {chrInfo}
  {showDups}
  {isStreaming}
  {sessId}
  {isQueryable}
/>