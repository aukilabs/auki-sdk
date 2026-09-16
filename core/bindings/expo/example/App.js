import React, { useEffect, useState } from 'react';
import { AppState, ScrollView, Text } from 'react-native';
import { runCases, fixture } from './cases';
import { runDomainDataCases, domainFixture } from './domain-data-cases';

const domainDataMode = process.env.EXPO_PUBLIC_AUKI_DOMAIN_DATA_TEST === '1';
const selectedCases = domainDataMode ? runDomainDataCases : runCases;
const selectedFixture = domainDataMode ? domainFixture : fixture;

let running;
const lines = [];
const listeners = new Set();
function report(line) { lines.push(line); for (const listener of listeners) listener([...lines]); }
export default function App() {
  const [output, setOutput] = useState(lines);
  useEffect(() => {
    listeners.add(setOutput);
    if (!running) {
      running = selectedCases(report).catch(async error => {
        report(`FAIL ${error.message}`);
        await selectedFixture('/__phase', { phase: 'failed', reason: error.message });
      });
    }
    const state = AppState.addEventListener('change', value => { report(`APPSTATE ${value}`); });
    return () => { listeners.delete(setOutput); state.remove(); };
  }, []);
  return <ScrollView contentContainerStyle={{ padding: 28, paddingTop: 64 }}>
    <Text accessibilityRole="header">{domainDataMode ? 'Expo Domain data tests' : 'ZITADEL Expo handoff tests'}</Text>
    <Text selectable>{output.length ? output.join('\n') : 'Running synthetic loopback tests…'}</Text>
  </ScrollView>;
}
