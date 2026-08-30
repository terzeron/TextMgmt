import "./App.css";
import { lazy, Suspense } from "react";
import { Route, Routes } from "react-router-dom";

import Navigation from "./Navigation";
import ErrorPage from "./ErrorPage";
import Home from "./Home";
import BookView from "./BookView";
import LatestBooks from "./LatestBooks";
import ComicsView from "./ComicsView";
import ViewSingle from "./ViewSingle";
import Admin from "./Admin";

const BookEdit = lazy(() => import("./BookEdit"));
const ComicsEdit = lazy(() => import("./ComicsEdit"));

export default function App() {
  return (
    <>
      <Routes>
        {/* Standalone full-screen viewer without Navigation */}
        {/* filePath는 쿼리 파라미터(?path=...)로 전달됨 */}
        <Route
          path="/viewer/:fileType/:entryId"
          element={<ViewSingle />}
          errorElement={<ErrorPage />}
        />
        {/* Main app with Navigation bar */}
        <Route path="/" element={<Navigation />} errorElement={<ErrorPage />}>
          <Route index element={<Home />} errorElement={<ErrorPage />} />
          <Route
            path="book-edit/*"
            element={
              <Suspense fallback={null}>
                <BookEdit />
              </Suspense>
            }
            errorElement={<ErrorPage />}
          />
          <Route
            path="book-view/*"
            element={<BookView />}
            errorElement={<ErrorPage />}
          />
          <Route
            path="book-latest"
            element={<LatestBooks />}
            errorElement={<ErrorPage />}
          />
          <Route
            path="comics-edit/*"
            element={
              <Suspense fallback={null}>
                <ComicsEdit />
              </Suspense>
            }
            errorElement={<ErrorPage />}
          />
          <Route
            path="comics-view/*"
            element={<ComicsView />}
            errorElement={<ErrorPage />}
          />
          <Route
            path="comics-latest"
            element={<LatestBooks contentType="comic" />}
            errorElement={<ErrorPage />}
          />
          <Route
            path="admin"
            element={<Admin />}
            errorElement={<ErrorPage />}
          />
        </Route>
        {/* Other standalone or fallback routes */}
      </Routes>
    </>
  );
}
