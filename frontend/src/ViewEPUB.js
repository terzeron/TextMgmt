import {useEffect, useRef, useState} from "react";
import {getUrlPrefix} from "./Common";
import {ReactReader} from "react-reader";
import Section, {SpineItem} from "../node_modules/epubjs/lib/section.js";

export default function ViewEPUB(props) {
  const [loading, setLoading] = useState(true);

  const renditionRef = useRef(null)
  const [url, setUrl] = useState("");
  const [location] = useState(null)
  const [locationChanged] = useState(null)

  useEffect(() => {
    if (renditionRef.current) {
      console.log(renditionRef.current);
    }

    console.log(`ViewEPUB: useEffect(${props})`, props);
    if (props && props.entryId) {
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      const url = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      setUrl(url);
      console.log(url);
      setLoading(false);
    }

    return () => {
      //console.log("cleanup");
      setUrl("");
    };
  }, []);

  if (loading) return <div>로딩 중...</div>;
  return (
    <div style={{height: "100vh"}}>
      <ReactReader
        location={location}
        locationChanged={locationChanged}
        url={url}
        key={url}
        getRendition={(rendition) => {
          const spine_get = rendition.book.spine.get.bind(rendition.book.spine);
          rendition.book.spine.get = function (target) {
            var t = spine_get(target);
            if (!t) {
              t = spine_get(undefined);
            }
            return t;
          }
        }}
      />
    </div>
  );
};