import React, { useCallback, useEffect, useRef } from 'react';
import CodeMirror, { ReactCodeMirrorRef } from '@uiw/react-codemirror';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView, keymap, lineNumbers, highlightActiveLineGutter, highlightActiveLine, drawSelection, rectangularSelection, crosshairCursor } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { syntaxHighlighting, defaultHighlightStyle, bracketMatching, foldGutter, indentOnInput, foldKeymap } from '@codemirror/language';
import { closeBrackets, closeBracketsKeymap, autocompletion, completionKeymap } from '@codemirror/autocomplete';
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search';
import { lintKeymap } from '@codemirror/lint';
import { Extension, EditorState } from '@codemirror/state';
import { invoke } from "@tauri-apps/api/core";

interface EditorProps {
  value: string;
  onChange: (val: string) => void;
  onSave: () => void;
  readOnly?: boolean;
}

const cerebroTheme = EditorView.theme({
  '&': {
    backgroundColor: '#0a0a0f',
    color: '#d4d4d8',
    height: '100%',
    fontSize: '14px',
  },
  '.cm-content': {
    caretColor: '#6366f1',
    fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", Menlo, Monaco, monospace',
    padding: '8px 0',
  },
  '.cm-cursor': {
    borderLeftColor: '#6366f1',
    borderLeftWidth: '2px',
  },
  '.cm-activeLine': {
    backgroundColor: '#12121a',
  },
  '.cm-activeLineGutter': {
    backgroundColor: '#12121a',
  },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': {
    backgroundColor: '#6366f140',
  },
  '.cm-gutters': {
    backgroundColor: '#0a0a0f',
    color: '#52525b',
    border: 'none',
    borderRight: '1px solid #1e1e2e',
  },
  '.cm-lineNumbers .cm-gutterElement': {
    minWidth: '3em',
    paddingRight: '8px',
  },
  '.cm-foldGutter': {
    width: '16px',
  },
  '.cm-scroller': {
    overflow: 'auto',
    lineHeight: '1.6',
  },
  '.cm-focused': {
    outline: 'none',
  },
  '.cm-matchingBracket': {
    backgroundColor: '#6366f130',
    color: '#a5b4fc',
    outline: '1px solid #6366f150',
  },
  '.cm-nonmatchingBracket': {
    backgroundColor: '#ef444430',
    color: '#fca5a5',
  },
  '.cm-searchMatch': {
    backgroundColor: '#6366f140',
    outline: '1px solid #6366f180',
  },
  '.cm-searchMatch.cm-searchMatch-selected': {
    backgroundColor: '#6366f160',
  },
  '.cm-panels': {
    backgroundColor: '#12121a',
    color: '#d4d4d8',
    borderBottom: '1px solid #2a2a3a',
  },
  '.cm-panels input, .cm-panels button': {
    fontFamily: 'inherit',
  },
  '.cm-tooltip': {
    backgroundColor: '#12121a',
    border: '1px solid #2a2a3a',
    color: '#d4d4d8',
  },
  '.cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]': {
    backgroundColor: '#6366f130',
    color: '#e0e7ff',
  },
}, { dark: true });

const cerebroHighlightStyle = syntaxHighlighting(defaultHighlightStyle, { fallback: true });

function useDebounce<T extends (...args: any[]) => void>(callback: T, delay: number): T {
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const callbackRef = useRef(callback);

  useEffect(() => {
    callbackRef.current = callback;
  }, [callback]);

  useEffect(() => {
    return () => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  return useCallback(
    ((...args: unknown[]) => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
      timeoutRef.current = setTimeout(() => {
        callbackRef.current(...args);
      }, delay);
    }) as T,
    [delay]
  );
}

const Editor: React.FC<EditorProps> = ({ value, onChange, onSave, readOnly = false }) => {
  const cmRef = useRef<ReactCodeMirrorRef>(null);
  const localValueRef = useRef<string>(value);
  const isExternalUpdate = useRef<boolean>(false);

  const debouncedOnChange = useDebounce((val: string) => {
    onChange(val);
  }, 300);

  useEffect(() => {
    if (value !== localValueRef.current) {
      isExternalUpdate.current = true;
      localValueRef.current = value;
    }
  }, [value]);

  const handleChange = useCallback(
    (val: string) => {
      localValueRef.current = val;
      debouncedOnChange(val);
    },
    [debouncedOnChange]
  );

  const saveKeymap = keymap.of([
    {
      key: 'Mod-s',
      run: () => {
        onSave();
        return true;
      },
      preventDefault: true,
    },
  ]);

  // Image paste and drag-drop handler
  const imageHandlers = EditorView.domEventHandlers({
    paste(event) {
      const items = event.clipboardData?.items;
      if (!items) return false;

      for (const item of items) {
        if (item.type.startsWith('image/')) {
          event.preventDefault();
          const file = item.getAsFile();
          if (!file) continue;

          const reader = new FileReader();
          reader.onload = async () => {
            const base64 = (reader.result as string).split(',')[1];
            try {
              const imagePath = await invoke<string>("save_image", {
                fileName: file.name || `paste_${Date.now()}.png`,
                data: base64,
              });
              const view = cmRef.current?.view;
              if (view) {
                const cursor = view.state.selection.main.head;
                view.dispatch({
                  changes: { from: cursor, insert: `\n![image](${imagePath})\n` }
                });
              }
            } catch (e) {
              console.error("Failed to save pasted image:", e);
            }
          };
          reader.readAsDataURL(file);
          return true;
        }
      }
      return false;
    },
    drop(event) {
      const files = event.dataTransfer?.files;
      if (!files || files.length === 0) return false;

      for (const file of files) {
        if (file.type.startsWith('image/')) {
          event.preventDefault();
          const reader = new FileReader();
          reader.onload = async () => {
            const base64 = (reader.result as string).split(',')[1];
            try {
              const imagePath = await invoke<string>("save_image", {
                fileName: file.name,
                data: base64,
              });
              const view = cmRef.current?.view;
              if (view) {
                const cursor = view.state.selection.main.head;
                view.dispatch({
                  changes: { from: cursor, insert: `\n![${file.name}](${imagePath})\n` }
                });
              }
            } catch (e) {
              console.error("Failed to save dropped image:", e);
            }
          };
          reader.readAsDataURL(file);
          return true;
        }
      }
      return false;
    },
  });

  const extensions: Extension[] = [
    cerebroTheme,
    cerebroHighlightStyle,
    lineNumbers(),
    highlightActiveLineGutter(),
    history(),
    foldGutter(),
    highlightActiveLine(),
    drawSelection(),
    rectangularSelection(),
    crosshairCursor(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    bracketMatching(),
    closeBrackets(),
    autocompletion(),
    highlightSelectionMatches(),
    EditorView.lineWrapping,
    markdown({ base: markdownLanguage, codeLanguages: languages }),
    keymap.of([
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...searchKeymap,
      ...historyKeymap,
      ...foldKeymap,
      ...completionKeymap,
      ...lintKeymap,
      indentWithTab,
    ]),
    saveKeymap,
    imageHandlers,
  ];

  if (readOnly) {
    extensions.push(EditorState.readOnly.of(true));
  }

  return (
    <div className="h-full w-full overflow-hidden" style={{ backgroundColor: '#0a0a0f' }}>
      <CodeMirror
        ref={cmRef}
        value={value}
        onChange={handleChange}
        extensions={extensions}
        theme={oneDark}
        basicSetup={false}
        className="h-full w-full"
        style={{
          height: '100%',
          fontSize: '14px',
        }}
      />
    </div>
  );
};

export default Editor;
