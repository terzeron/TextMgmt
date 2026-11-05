//! 내장 규칙. 강한 IOC(침해 지표)만 둔다: 정상 문서에 나올 이유가 거의 없는 문자열 조합이다.
//! `--builtin-rules`로 켠다. 규칙은 파일의 원시 바이트에 적용되므로 압축된 내용(ZIP 멤버, PDF·OLE 스트림)은 보지 못한다.
//! 규칙 문법과 YARA와의 차이는 `docs/superpowers/plans/2026-10-30-bookdoctor-p5.md`에 있다.

/// 내장 규칙 원문.
pub const BUILTIN: &str = r#"
// EICAR 표준 안티바이러스 시험 문자열. 악성 코드가 아니라 탐지기 점검용이다.
// 저장소 파일이 백신에 걸리지 않도록 두 조각으로 나눠 두 조각이 모두 있을 때만 일치시킨다.
rule EICAR_TEST_FILE {
  meta:
    severity = "critical"
    description = "EICAR 표준 안티바이러스 시험 문자열(악성 코드가 아니라 탐지기 점검용)"
  strings:
    $first = "X5O!P%@AP[4\\PZX54(P^)7CC)7}$"
    $second = "EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*"
  condition:
    $first and $second
}

// 인코딩된 명령을 받는 PowerShell 실행줄. `-Encoding` 같은 옵션과 구별하려고 공백이 붙은 `-enc `와 전체 이름만 쓴다.
rule POWERSHELL_ENCODED_COMMAND {
  meta:
    severity = "high"
    description = "PowerShell이 인코딩된 명령을 실행한다(-enc, -EncodedCommand)"
  strings:
    $ps = "powershell" nocase ascii wide
    $enc = "-enc " nocase ascii wide
    $ec = "-ec " nocase ascii wide
    $full = "-encodedcommand" nocase ascii wide
  condition:
    $ps and ($enc or $ec or $full)
}

// 내려받아 실행하는 PowerShell 관용구.
rule POWERSHELL_DOWNLOAD_CRADLE {
  meta:
    severity = "high"
    description = "PowerShell이 Net.WebClient로 내려받는다(DownloadString, DownloadFile)"
  strings:
    $client = "Net.WebClient" nocase ascii wide
    $string = "DownloadString" nocase ascii wide
    $file = "DownloadFile" nocase ascii wide
  condition:
    $client and ($string or $file)
}

// 원격 주소를 실행하거나 내려받는 Windows 기본 도구(LOLBin) 사용.
rule LOLBIN_REMOTE_EXEC {
  meta:
    severity = "high"
    description = "mshta·regsvr32·certutil·bitsadmin으로 원격 내용을 실행하거나 내려받는다"
  strings:
    $mshta = "mshta" nocase ascii wide
    $http = "http://" nocase ascii wide
    $https = "https://" nocase ascii wide
    $regsvr = "regsvr32" nocase ascii wide
    $remote_scriptlet = "/i:http" nocase ascii wide
    $certutil = "certutil" nocase ascii wide
    $urlcache = "-urlcache" nocase ascii wide
    $bitsadmin = "bitsadmin" nocase ascii wide
    $transfer = "/transfer" nocase ascii wide
  condition:
    ($mshta and ($http or $https)) or ($regsvr and $remote_scriptlet) or ($certutil and $urlcache) or ($bitsadmin and $transfer and ($http or $https))
}

// MZ 헤더(`MZ\x90\0\x03\0\0\0\x04\0\0\0` 등)를 base64로 인코딩한 문자열. 스크립트에 실려 오는 실행 파일의 흔한 모양이다.
rule BASE64_PE_HEADER {
  meta:
    severity = "high"
    description = "base64로 인코딩한 PE(MZ) 헤더가 들어 있다"
  strings:
    $a = "TVqQAAMAAAAEAAAA" ascii wide
    $b = "TVpQAAIAAAAEAA8A" ascii wide
    $c = "TVqAAAEAAAAEABAA" ascii wide
  condition:
    any of them
}

// MZ 헤더를 16진 문자열로 쓴 것.
rule HEX_PE_HEADER {
  meta:
    severity = "high"
    description = "16진 문자열로 쓴 PE(MZ) 헤더가 들어 있다"
  strings:
    $mz = "4d5a9000030000000400" nocase ascii wide
  condition:
    $mz
}

// WScript.Shell로 명령을 실행한다. 정상 관리 스크립트에도 흔해 medium이다.
rule WSCRIPT_SHELL_RUN {
  meta:
    severity = "medium"
    description = "WScript.Shell 객체로 명령을 실행한다(.Run, .Exec)"
  strings:
    $shell = "WScript.Shell" nocase ascii wide
    $run = ".Run" nocase ascii wide
    $exec = ".Exec" nocase ascii wide
  condition:
    $shell and ($run or $exec)
}
"#;
