import React, { useEffect, useState } from 'react';
import { AppState, ScrollView, Text } from 'react-native';
import { runCases, fixture } from './cases';

let running;
const lines = [];
const listeners = new Set();
function report(line) { lines.push(line); for (const listener of listeners) listener([...lines]); }
export default function App() {
  const [output, setOutput] = useState(lines);
  useEffect(() => {
    listeners.add(setOutput);
    if (!running) {
      running = runCases(report).catch(async error => {
        report(`FAIL ${error.message}`);
        await fixture('/__phase', { phase: 'failed', reason: error.message });
      });
    }
    const state = AppState.addEventListener('change', value => { report(`APPSTATE ${value}`); });
    return () => { listeners.delete(setOutput); state.remove(); };
  }, []);
  return <ScrollView contentContainerStyle={{ padding: 28, paddingTop: 64 }}>
    <Text accessibilityRole="header">ZITADEL Expo handoff tests</Text>
    <Text selectable>{output.length ? output.join('\n') : 'Running synthetic loopback tests…'}</Text>
  </ScrollView>;
}
