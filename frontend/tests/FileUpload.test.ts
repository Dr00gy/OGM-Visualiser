import { render, fireEvent } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import FileUpload from '../src/routes/FileUpload.svelte';

describe('FileUpload', () => {
  it('accepts files via hidden input', async () => {
    const { container } = render(FileUpload);
    const uploadArea = container.querySelector('.upload-area')!;
    const button = uploadArea.querySelector('button.upload-button')!;
    await fireEvent.click(button);

    const input = uploadArea.querySelector('input[type="file"]') as HTMLInputElement;
    const file = new File(['content'], 'test.xmap', { type: 'text/plain' });
    await fireEvent.change(input, { target: { files: [file] } });// jsdom-friendly change event

    expect(input.files?.[0].name).toBe('test.xmap');
  });
});
