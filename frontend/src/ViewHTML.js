import {useEffect, useRef, useState} from "react";
import {getUrlPrefix} from "./Common";

export default function ViewTXT(props) {
  const ref = useRef(null);
  const [iframeHeight, setIframeHeight] = useState(0);
  const [url, setUrl] = useState("");

  useEffect(() => {
    console.log(`ViewHTML: useEffect(${props})`, props);
    setIframeHeight(document.body.scrollHeight);

    if (props && props.entryId) {
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      const url = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      console.log(url);
      setUrl(url);
      console.log(ref.current);
    }

    return () => {
      setUrl('')
      setIframeHeight(0)
    };
  }, [props]);

  return (
    <iframe src={url} ref={ref} style={{display: 'block', width: '100%', height: iframeHeight, overflow: 'visible'}}/>
  );
}


