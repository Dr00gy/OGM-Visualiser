declare module '$app/environment' {
    export const browser: boolean;
}

declare module '*.svg' {
    const src: string;
    export default src;
}