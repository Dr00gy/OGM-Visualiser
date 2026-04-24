import { writable } from 'svelte/store';
import { browser } from '$app/environment';

function createDarkModeStore() {
  const getInitialValue = () => {
    if (!browser) return false;

    const stored = localStorage.getItem('darkMode');
    if (stored !== null) {
      return stored === 'true';
    }

    return window.matchMedia('(prefers-color-scheme: dark)').matches;
  };

  const { subscribe, set, update } = writable(getInitialValue());

  return {
    subscribe,

    toggle: () => {
      update(value => {
        const newValue = !value;
        if (browser) {
          localStorage.setItem('darkMode', String(newValue));
          document.documentElement.classList.toggle('dark', newValue);
        }
        return newValue;
      });
    },

    set: (value: boolean) => {
      if (browser) {
        localStorage.setItem('darkMode', String(value));
        document.documentElement.classList.toggle('dark', value);
      }
      set(value);
    }
  };
}

export const darkMode = createDarkModeStore();