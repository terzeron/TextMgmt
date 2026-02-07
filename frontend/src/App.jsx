import './App.css'
import {Route, Routes} from 'react-router-dom';

import Navigation from "./Navigation";
import ErrorPage from "./ErrorPage";
import Home from "./Home";
import Edit from "./Edit";
import View from "./View";
import ViewSingle from "./ViewSingle";
import Admin from "./Admin";

export default function App() {
    return (
        <>
            <Routes>
                {/* Standalone full-screen viewer without Navigation */}
                {/* filePath는 쿼리 파라미터(?path=...)로 전달됨 */}
                <Route path="/viewer/:fileType/:entryId" element={<ViewSingle/>} errorElement={<ErrorPage/>}/>
                {/* `/edit/:category/:bookId` is nested under Navigation to provide search context */}
                {/* Main app with Navigation bar */}
                <Route path="/" element={<Navigation/>} errorElement={<ErrorPage/>}>
                    <Route index element={<Home/>} errorElement={<ErrorPage/>}/>
                    <Route path="edit" element={<Edit/>} errorElement={<ErrorPage/>}/>
                    <Route path="view" element={<View/>} errorElement={<ErrorPage/>}/>
                    <Route path="admin" element={<Admin/>} errorElement={<ErrorPage/>}/>
                    {/* Nested routes handled by main app */}
                    <Route path="edit/:category/:bookId" element={<Edit/>} errorElement={<ErrorPage/>}/>
                    <Route path="view/:category/:bookId" element={<View/>} errorElement={<ErrorPage/>}/>
                </Route>
                {/* Other standalone or fallback routes */}
            </Routes>
        </>
    );
}