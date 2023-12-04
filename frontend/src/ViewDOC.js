import {useEffect, useRef, useState} from "react";
import {getUrlPrefix} from "./Common";

export default function ViewDOC(props) {
  const [url, setUrl] = useState("");
  const [iframeHeight, setIframeHeight] = useState(0);
  const ref = useRef(null);

  useEffect(() => {
    console.log(`ViewDOC: useEffect()`, props);
    setIframeHeight(document.body.scrollHeight);
    if (props && props.entryId) {
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      const uri = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + fileName;
      const wordViewerUrlPrefix = 'https://view.officeapps.live.com/op/embed.aspx?src=';
      setUrl(wordViewerUrlPrefix + encodeURIComponent(uri));
      console.log(wordViewerUrlPrefix + encodeURIComponent(uri));
    }
  }, []);

  return (
    url && <iframe src={url} ref={ref} style={{display: 'block', width: '100%', height: iframeHeight, overflow: 'visible'}}/>
  );
}
