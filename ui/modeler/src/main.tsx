import React from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';

import { App } from './App';
import './styles.css';

const root = document.getElementById('root');

if (!root) {
  throw new Error('Modeler root element was not found');
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <BrowserRouter basename="/modeler-app">
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);
