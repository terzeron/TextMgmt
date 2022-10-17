import './App.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Route, Routes} from 'react-router-dom';
import React from "react";
import Home from "./Home";
import Edit from "./Edit";
import View from "./View";
import Navigation from "./Navigation";
import ViewSingle from "./ViewSingle";
import ErrorPage from "./ErrorPage";

function App() {
  return (
    <div className="App">
      <Routes>
        <Route path="/" element={<Navigation/>} errorElement={<ErrorPage />}>
          <Route index element={<Home/>} errorElement={<ErrorPage />}/>
          <Route path="edit" element={<Edit/>} errorElement={<ErrorPage />}/>
          <Route path="view" element={<View/>} errorElement={<ErrorPage />}/>

        </Route>
        <Route path="/view/:dirName/:fileName" element={<ViewSingle/>} errorElement={<ErrorPage />}/>
      </Routes>
    </div>
  );
}

export default App;
