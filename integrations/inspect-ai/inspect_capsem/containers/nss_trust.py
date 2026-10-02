"""Trust a CA certificate in the NSS user DB ($HOME/.pki/nssdb), for images without certutil.

This file runs inside the container (`python3 -c <source> <ca.pem>`), not in the provider:
Chromium on Linux trusts extra roots from this DB, not from /etc/ssl/certs. It uses only
the standard library and the image's libnss3, which Chromium needs anyway.
"""

import ctypes
import ctypes.util
import ssl
import sys
from pathlib import Path


class _Trust(ctypes.Structure):
    _fields_ = [("ssl", ctypes.c_uint), ("email", ctypes.c_uint), ("objsign", ctypes.c_uint)]


class _SECItem(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int),
        ("data", ctypes.POINTER(ctypes.c_ubyte)),
        ("len", ctypes.c_uint),
    ]


def trust_ca(pem_path: str) -> None:
    nss = ctypes.CDLL(ctypes.util.find_library("nss3") or "libnss3.so")
    vp = ctypes.c_void_p
    nss.PK11_GetInternalKeySlot.restype = vp
    nss.CERT_GetDefaultCertDB.restype = vp
    nss.CERT_NewTempCertificate.restype = vp
    nss.CERT_NewTempCertificate.argtypes = [
        vp,
        ctypes.POINTER(_SECItem),
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_int,
    ]
    nss.PK11_NeedUserInit.argtypes = [vp]
    nss.PK11_InitPin.argtypes = [vp, ctypes.c_char_p, ctypes.c_char_p]
    nss.PK11_ImportCert.argtypes = [vp, vp, ctypes.c_ulong, ctypes.c_char_p, ctypes.c_int]
    nss.CERT_ChangeCertTrust.argtypes = [vp, vp, ctypes.POINTER(_Trust)]
    nss.CERT_DestroyCertificate.argtypes = [vp]
    nss.PK11_FreeSlot.argtypes = [vp]

    db = Path.home() / ".pki" / "nssdb"
    db.mkdir(mode=0o700, parents=True, exist_ok=True)
    if nss.NSS_InitReadWrite(f"sql:{db}".encode()) != 0:
        raise SystemExit(f"capsem: cannot open NSS DB {db}")
    slot = nss.PK11_GetInternalKeySlot()
    if nss.PK11_NeedUserInit(slot):
        nss.PK11_InitPin(slot, None, b"")
    der = ssl.PEM_cert_to_DER_cert(Path(pem_path).read_text())
    item = _SECItem(0, (ctypes.c_ubyte * len(der)).from_buffer_copy(der), len(der))
    certdb = nss.CERT_GetDefaultCertDB()
    cert = nss.CERT_NewTempCertificate(certdb, ctypes.byref(item), None, 0, 1)
    if not cert:
        raise SystemExit("capsem: cannot decode the CA certificate")
    trust = _Trust()
    if (
        nss.PK11_ImportCert(slot, cert, 0, b"capsem-ca", 0) != 0
        or nss.CERT_DecodeTrustString(ctypes.byref(trust), b"C,,") != 0
        or nss.CERT_ChangeCertTrust(certdb, cert, ctypes.byref(trust)) != 0
    ):
        raise SystemExit("capsem: cannot trust the CA in the NSS DB")
    nss.CERT_DestroyCertificate(cert)
    nss.PK11_FreeSlot(slot)
    nss.NSS_Shutdown()


if __name__ == "__main__":
    trust_ca(sys.argv[1])
