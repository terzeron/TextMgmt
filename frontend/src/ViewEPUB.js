import {useEffect, useRef, useState, Suspense} from "react";
import {getUrlPrefix} from "./Common";
import {ReactReader} from "react-reader";

export default function ViewEPUB(props) {
  const renditionRef = useRef(null)
  const [url, setUrl] = useState("");
  const [location] = useState('')
  const [locationChanged] = useState(null);

  useEffect(() => {
    if (renditionRef.current) {
      console.log(renditionRef.current);
    }

    console.log(`ViewEPUB: useEffect()`, props);
    if (props && props.entryId) {
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      const url = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      setUrl(url);
      console.log(url);
    }

    return () => {
      setUrl("");
    };
  }, []);

  return (
    <div style={{height: "100vh"}}>
      <Suspense fallback={<div className="loading">로딩 중...</div>}>
        <ReactReader
          location={location}
          locationChanged={locationChanged}
          url={url}
          key={url}
          getRendition={(rendition) => {
            const spine_get = rendition.book.spine.get.bind(rendition.book.spine);
            rendition.book.spine.get = function (target) {
              let t = spine_get(target);
              if (!t) {
                t = spine_get(undefined);
              }
              return t;
            }
          }}
        />
      </Suspense>
    </div>
  );
}