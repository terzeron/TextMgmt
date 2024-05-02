import './App.css'
import {Route, Routes} from 'react-router-dom';

import Navigation from "./Navigation";
import ErrorPage from "./ErrorPage";
import Home from "./Home";
import Edit from "./Edit";
import View from "./View";
import ViewSingle from "./ViewSingle";

export default function App() {
    return (
        <>
            <Routes>
                <Route path="/" element={<Navigation/>} errorElement={<ErrorPage/>}>
                    <Route index element={<Home/>} errorElement={<ErrorPage/>}/>
                    <Route path="edit" element={<Edit/>} errorElement={<ErrorPage/>}/>
                    <Route path="view" element={<View/>} errorElement={<ErrorPage/>}/>
                </Route>
                <Route path="/view/:fileType/:entryId/:filePath" element={<ViewSingle/>} errorElement={<ErrorPage/>}/>
            </Routes>
        </>
    );
}