import {useEffect, useState} from "react";
import {getUrlPrefix} from "./Common";

export default function ViewImage(props) {
  const [url, setUrl] = useState("");

  useEffect(() => {
    console.log(`ViewImage: useEffect(${props})`, props);

    if (props && props.entryId) {
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      const url = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      console.log(url);
      setUrl(url);
    }

    return () => {
      setUrl('')
    };
  }, [props]);

  return (
    <img src={url}/>
  );
}


