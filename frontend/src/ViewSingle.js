import './ViewSingle.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useRef, useState} from "react";
import {useParams} from "react-router-dom";

import ViewPDF from "./ViewPDF";
import ViewEPUB from "./ViewEPUB";
import ViewTXT from "./ViewTXT";
import ViewHTML from './ViewHTML';
import ViewRTF from './ViewRTF';
import ViewImage from "./ViewImage";

export default function ViewSingle(props) {
  const {dirName, fileName} = useParams();
  const [entryId, setEntryId] = useState("");
  const [lineCount, setLineCount] = useState(0);
  const [errorMessage, setErrorMessage] = useState("");

  useEffect(() => {
    if (props) {
      console.log(`ViewSingle: useEffect() props=${JSON.stringify(props)}`);
    } else {
      console.log(`ViewSingle: useEffect() dirName=${dirName}, fileName=${fileName})`);
    }

    if (props.entryId) {
      setEntryId(props.entryId);
      setLineCount(props.lineCount);
    }
    if (dirName && fileName) {
      setEntryId(dirName + "/" + fileName);
    }

    return () => {
      setEntryId("");
    }
  }, []);

  return (
    <div>
      {
        entryId && entryId.endsWith(".pdf") && <ViewPDF entryId={entryId}/>
      }
      {
        entryId && entryId.endsWith(".epub") && <ViewEPUB entryId={entryId}/>
      }
      {
        entryId && entryId.endsWith(".txt") && <ViewTXT entryId={entryId} lineCount={lineCount}/>
      }
      {
        entryId && entryId.endsWith(".html") && <ViewHTML entryId={entryId}/>
      }
      {
        entryId && entryId.endsWith(".rtf") && <ViewRTF entryId={entryId}/>
      }      {
        entryId && (entryId.endsWith(".jpg") || entryId.endsWith(".gif") || entryId.endsWith(".png")) && <ViewImage entryId={entryId}/>
      }
    </div>
  );
}
