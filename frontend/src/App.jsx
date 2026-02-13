import './App.css'
import {Route, Routes} from 'react-router-dom';

import Navigation from "./Navigation";
import ErrorPage from "./ErrorPage";
import Home from "./Home";
import BookEdit from "./BookEdit";
import BookView from "./BookView";
import ComicsEdit from "./ComicsEdit";
import ComicsView from "./ComicsView";
import ViewSingle from "./ViewSingle";
import Admin from "./Admin";

export default function App() {
    return (
        <>
            <Routes>
                {/* Standalone full-screen viewer without Navigation */}
                {/* filePath는 쿼리 파라미터(?path=...)로 전달됨 */}
                <Route path="/viewer/:fileType/:entryId" element={<ViewSingle/>} errorElement={<ErrorPage/>}/>
                {/* Main app with Navigation bar */}
                <Route path="/" element={<Navigation/>} errorElement={<ErrorPage/>}>
                    <Route index element={<Home/>} errorElement={<ErrorPage/>}/>
                    <Route path="book-edit/*" element={<BookEdit/>} errorElement={<ErrorPage/>}/>
                    <Route path="book-view/*" element={<BookView/>} errorElement={<ErrorPage/>}/>
                    <Route path="comics-edit/*" element={<ComicsEdit/>} errorElement={<ErrorPage/>}/>
                    <Route path="comics-view/*" element={<ComicsView/>} errorElement={<ErrorPage/>}/>
                    <Route path="admin" element={<Admin/>} errorElement={<ErrorPage/>}/>
                </Route>
                {/* Other standalone or fallback routes */}
            </Routes>
        </>
    );
}