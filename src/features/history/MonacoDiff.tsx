import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { editor } from 'monaco-editor/editor/editor.api.js';
import type { DiffContent, RepositoryView } from '../../contracts/history';
import type { MonacoLease } from './monacoWorker';

type TextDiff = Extract<DiffContent, { kind: 'text' }>;
export type MonacoDiffProps = Pick<TextDiff, 'original' | 'modified'> & {
  language?: TextDiff['language'];
  layout: RepositoryView['diffLayout'];
};
type Resources = MonacoLease & {
  editor: editor.IStandaloneDiffEditor;
  original: editor.ITextModel;
  modified: editor.ITextModel;
};

function supportedLanguage(api: MonacoLease['api'], language: MonacoDiffProps['language']) {
  return language && api.languages.getLanguages().some(value => value.id === language) ? language : 'plaintext';
}

function update(resources: Resources, props: MonacoDiffProps) {
  if (resources.original.getValue() !== props.original) resources.original.setValue(props.original);
  if (resources.modified.getValue() !== props.modified) resources.modified.setValue(props.modified);
  const language = supportedLanguage(resources.api, props.language);
  for (const model of [resources.original, resources.modified]) {
    if (model.getLanguageId() !== language) resources.api.editor.setModelLanguage(model, language);
  }
  resources.editor.updateOptions({ renderSideBySide: props.layout === 'sideBySide' });
}

export type MonacoDiffLoader = (onWorkerFailure: () => void) => Promise<MonacoLease>;

// The loader is fixed for a component's lifetime. Production keeps the static
// local lazy import below; consumers cannot supply worker URLs or module paths.
export function createMonacoDiff(loadMonaco: MonacoDiffLoader) {
  /** Readonly text renderer. The caller owns file selection and unavailable DTOs. */
  return function MonacoDiff(props: MonacoDiffProps) {
    const host = useRef<HTMLDivElement>(null);
    const latest = useRef(props);
    const resources = useRef<Resources | null>(null);
    const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');

    useLayoutEffect(() => {
      latest.current = props;
      if (resources.current) update(resources.current, props);
    }, [props.original, props.modified, props.language, props.layout]);

    useEffect(() => {
      let closed = false;
      let failed = false;
      let release: (() => void) | undefined;
      let diff: editor.IStandaloneDiffEditor | undefined;
      let original: editor.ITextModel | undefined;
      let modified: editor.ITextModel | undefined;
      let observer: ResizeObserver | undefined;
      const dispose = () => {
        // An earlier StrictMode effect may finish loading after a newer effect.
        if (diff && resources.current?.editor === diff) resources.current = null;
        observer?.disconnect(); observer = undefined;
        diff?.dispose(); diff = undefined;
        original?.dispose(); original = undefined;
        modified?.dispose(); modified = undefined;
        release?.(); release = undefined;
      };
      const fail = () => {
        if (closed) return;
        failed = true;
        dispose();
        setStatus('error');
      };
      setStatus('loading');
      void Promise.resolve().then(() => loadMonaco(fail)).then(lease => {
        release = lease.release;
        if (closed || failed || !host.current) { dispose(); return; }
        const value = latest.current;
        const language = supportedLanguage(lease.api, value.language);
        original = lease.api.editor.createModel(value.original, language);
        modified = lease.api.editor.createModel(value.modified, language);
        diff = lease.api.editor.createDiffEditor(host.current, {
          theme: 'vs-dark',
          readOnly: true,
          domReadOnly: true,
          originalEditable: false,
          ignoreTrimWhitespace: false,
          tabFocusMode: true,
          renderSideBySide: value.layout === 'sideBySide',
          useInlineViewWhenSpaceIsLimited: false,
          originalAriaLabel: 'Original text (read only)',
          modifiedAriaLabel: 'Modified text (read only)',
          minimap: { enabled: false },
          automaticLayout: false,
        });
        if (failed) { dispose(); return; }
        diff.setModel({ original, modified });
        resources.current = { ...lease, editor: diff, original, modified };
        observer = new ResizeObserver(() => { if (!closed && !failed) diff?.layout(); });
        observer.observe(host.current);
        diff.layout();
        setStatus('ready');
      }).catch(fail);
      return () => { closed = true; dispose(); };
    }, []);

    return <section className="monaco-diff" aria-label="Read-only text diff" aria-busy={status === 'loading'} style={{ position: 'relative', width: '100%', height: '100%', minHeight: 160 }}>
      {status === 'loading' && <p role="status">Loading text diff…</p>}
      {status === 'error' && <p role="alert">Text diff could not be loaded.</p>}
      <div ref={host} className="monaco-diff-editor" style={{ width: '100%', height: '100%', minHeight: 160, visibility: status === 'ready' ? 'visible' : 'hidden' }} />
    </section>;
  };
}

const loadDefaultMonaco: MonacoDiffLoader = async onWorkerFailure => {
  const { loadMonaco } = await import('./monacoWorker');
  return loadMonaco(onWorkerFailure);
};
export const MonacoDiff = createMonacoDiff(loadDefaultMonaco);
