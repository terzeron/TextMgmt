import './App.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Route, Routes} from 'react-router-dom';
import React, {useEffect} from "react";
import Home from "./Home";
import Edit from "./Edit";
import View from "./View";
import Navigation from "./Navigation";
import ViewSingle from "./ViewSingle";
import ErrorPage from "./ErrorPage";
import {jsonGetReq} from "./Common";

function App() {
  useEffect(() => {
    // intentional API calls before rendering of Edit and View component
    console.log(`call /somedir for partial tree data`)
    const someDirListUrl = '/somedirs';
    jsonGetReq(someDirListUrl, (result) => {
    }, (error) => {
    });
    console.log("call /dirs for treeData");
    const fullDirListUrl = '/dirs';
    jsonGetReq(fullDirListUrl, (result) => {
    }, (error) => {
    });
    console.log("call /topdirs for other dir list")
    const topDirListUrl = '/topdirs';
    jsonGetReq(topDirListUrl, (result) => {
    }, (error) => {
    });
  });

  return (
    <div className="App">
      <Routes>
        <Route path="/" element={<Navigation/>} errorElement={<ErrorPage/>}>
          <Route index element={<Home/>} errorElement={<ErrorPage/>}/>
          <Route path="edit" element={<Edit/>} errorElement={<ErrorPage/>}/>
          <Route path="view" element={<View/>} errorElement={<ErrorPage/>}/>
        </Route>
        <Route path="/view/:dirName/:fileName" element={<ViewSingle/>} errorElement={<ErrorPage/>}/>
      </Routes>
    </div>
  );
}

export default App;
