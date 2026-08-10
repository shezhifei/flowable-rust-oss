import { Route, Routes } from 'react-router-dom';

import { ModelerWorkspace } from './modeler/ModelerWorkspace';

export function App() {
  return (
    <Routes>
      <Route path="models/:modelId/bpmn" element={<ModelerWorkspace />} />
      <Route path="*" element={<ModelerWorkspace />} />
    </Routes>
  );
}
