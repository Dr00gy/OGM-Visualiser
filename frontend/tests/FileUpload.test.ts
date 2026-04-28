import { render, fireEvent } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import FileUpload from '../src/routes/FileUpload.svelte';

function mkRelFile(name: string, webkitRelativePath: string) {
  const f = new File(['content'], name, { type: 'text/plain' });
  // Svelte code reads (f as any).webkitRelativePath
  Object.defineProperty(f, 'webkitRelativePath', {
    value: webkitRelativePath,
    configurable: true,
  });
  return f;
}

describe('FileUpload', () => {
  it('accepts files via the hidden directory input (zone 1)', async () => {
    const { container } = render(FileUpload);

    const zone0 = container.querySelectorAll('.zone')[0] as HTMLElement | undefined;
    expect(zone0).toBeTruthy();

    const input = zone0!.querySelector('input[type="file"]') as HTMLInputElement | null;
    expect(input).toBeTruthy();

    // Build a minimal FileList-like object. @testing-library will set `target.files`,
    // but in jsdom `input.files` isn’t always writable; defining it is more robust.
    const files = [
      // a contig xmap in the expected contig path
      mkRelFile(
        'contig1.xmap',
        'GenomeA/assembly/output/contigs/exp_refineFinal1/alignmol/merge/contig1.xmap',
      ),
      // refine final at root (current implementation requires parts.length === 2)
      mkRelFile('exp_refineFinal1.xmap', 'GenomeA/exp_refineFinal1.xmap'),
    ];

    Object.defineProperty(input!, 'files', {
      value: files,
      configurable: true,
    });

    await fireEvent.change(input!);

    // After change, the zone should switch from empty state to filled state
    expect(zone0!.classList.contains('filled')).toBe(true);

    // UI should show the directory name and refinefinal indicator
    expect(zone0!.textContent).toContain('GenomeA');
    expect(zone0!.textContent).toContain('✓ exp_refineFinal1.xmap');
    expect(zone0!.textContent).toContain('sequence file');
  });
});