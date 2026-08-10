import { Route, Routes } from 'react-router-dom';

import { DmnWorkspace } from './dmn/ui/DmnWorkspace';
import { FormWorkspace } from './form/ui/FormWorkspace';
import { ModelerWorkspace } from './modeler/ModelerWorkspace';

export function App() {
  return (
    <Routes>
      <Route path="models/:modelId/form" element={<FormWorkspace />} />
      <Route path="models/:modelId/dmn" element={<DmnWorkspace />} />
      <Route path="models/:modelId/bpmn" element={<ModelerWorkspace />} />
      <Route path="*" element={<ModelerWorkspace />} />
    </Routes>
  );
}
