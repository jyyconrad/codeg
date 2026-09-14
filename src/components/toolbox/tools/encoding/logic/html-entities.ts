import { EncodingError } from "./error"

const NAMED_ENTITIES = buildNamedEntities()

const ENTITY_RE = /&(#x[0-9a-fA-F]+|#\d+|[a-zA-Z][a-zA-Z0-9]*);/g

export function escapeHtml(input: string): string {
  return input.replace(/[&<>"']/g, (ch) => {
    switch (ch) {
      case "&":
        return "&amp;"
      case "<":
        return "&lt;"
      case ">":
        return "&gt;"
      case '"':
        return "&quot;"
      default:
        return "&#39;"
    }
  })
}

export function unescapeHtml(input: string): string {
  return input.replace(ENTITY_RE, (entity, body: string) => {
    if (body.startsWith("#")) {
      return fromNumericEntity(entity, body)
    }
    const named = NAMED_ENTITIES.get(body)
    if (named == null) {
      throw new EncodingError(`Unknown HTML entity: ${entity}`)
    }
    return named
  })
}

function fromNumericEntity(entity: string, body: string): string {
  const hex = body[1] === "x" || body[1] === "X"
  const digits = body.slice(hex ? 2 : 1)
  const code = Number.parseInt(digits, hex ? 16 : 10)
  if (
    !Number.isInteger(code) ||
    code < 0 ||
    code > 0x10ffff ||
    (code >= 0xd800 && code <= 0xdfff)
  ) {
    throw new EncodingError(`Invalid HTML entity: ${entity}`)
  }
  return String.fromCodePoint(code)
}

function buildNamedEntities(): Map<string, string> {
  const map = new Map<string, string>([
    ["amp", "&"],
    ["lt", "<"],
    ["gt", ">"],
    ["quot", '"'],
    ["apos", "'"],
  ])

  const latin1 = [
    "nbsp",
    "iexcl",
    "cent",
    "pound",
    "curren",
    "yen",
    "brvbar",
    "sect",
    "uml",
    "copy",
    "ordf",
    "laquo",
    "not",
    "shy",
    "reg",
    "macr",
    "deg",
    "plusmn",
    "sup2",
    "sup3",
    "acute",
    "micro",
    "para",
    "middot",
    "cedil",
    "sup1",
    "ordm",
    "raquo",
    "frac14",
    "frac12",
    "frac34",
    "iquest",
    "Agrave",
    "Aacute",
    "Acirc",
    "Atilde",
    "Auml",
    "Aring",
    "AElig",
    "Ccedil",
    "Egrave",
    "Eacute",
    "Ecirc",
    "Euml",
    "Igrave",
    "Iacute",
    "Icirc",
    "Iuml",
    "ETH",
    "Ntilde",
    "Ograve",
    "Oacute",
    "Ocirc",
    "Otilde",
    "Ouml",
    "times",
    "Oslash",
    "Ugrave",
    "Uacute",
    "Ucirc",
    "Uuml",
    "Yacute",
    "THORN",
    "szlig",
    "agrave",
    "aacute",
    "acirc",
    "atilde",
    "auml",
    "aring",
    "aelig",
    "ccedil",
    "egrave",
    "eacute",
    "ecirc",
    "euml",
    "igrave",
    "iacute",
    "icirc",
    "iuml",
    "eth",
    "ntilde",
    "ograve",
    "oacute",
    "ocirc",
    "otilde",
    "ouml",
    "divide",
    "oslash",
    "ugrave",
    "uacute",
    "ucirc",
    "uuml",
    "yacute",
    "thorn",
    "yuml",
  ]
  latin1.forEach((name, index) => {
    map.set(name, String.fromCharCode(160 + index))
  })

  const extra: [string, number][] = [
    ["fnof", 402],
    ["Alpha", 913],
    ["Beta", 914],
    ["Gamma", 915],
    ["Delta", 916],
    ["Epsilon", 917],
    ["Zeta", 918],
    ["Eta", 919],
    ["Theta", 920],
    ["Iota", 921],
    ["Kappa", 922],
    ["Lambda", 923],
    ["Mu", 924],
    ["Nu", 925],
    ["Xi", 926],
    ["Omicron", 927],
    ["Pi", 928],
    ["Rho", 929],
    ["Sigma", 931],
    ["Tau", 932],
    ["Upsilon", 933],
    ["Phi", 934],
    ["Chi", 935],
    ["Psi", 936],
    ["Omega", 937],
    ["alpha", 945],
    ["beta", 946],
    ["gamma", 947],
    ["delta", 948],
    ["epsilon", 949],
    ["zeta", 950],
    ["eta", 951],
    ["theta", 952],
    ["iota", 953],
    ["kappa", 954],
    ["lambda", 955],
    ["mu", 956],
    ["nu", 957],
    ["xi", 958],
    ["omicron", 959],
    ["pi", 960],
    ["rho", 961],
    ["sigmaf", 962],
    ["sigma", 963],
    ["tau", 964],
    ["upsilon", 965],
    ["phi", 966],
    ["chi", 967],
    ["psi", 968],
    ["omega", 969],
    ["thetasym", 977],
    ["upsih", 978],
    ["piv", 982],
    ["bull", 8226],
    ["hellip", 8230],
    ["prime", 8242],
    ["Prime", 8243],
    ["oline", 8254],
    ["frasl", 8260],
    ["weierp", 8472],
    ["image", 8465],
    ["real", 8476],
    ["trade", 8482],
    ["alefsym", 8501],
    ["larr", 8592],
    ["uarr", 8593],
    ["rarr", 8594],
    ["darr", 8595],
    ["harr", 8596],
    ["crarr", 8629],
    ["lArr", 8656],
    ["uArr", 8657],
    ["rArr", 8658],
    ["dArr", 8659],
    ["hArr", 8660],
    ["forall", 8704],
    ["part", 8706],
    ["exist", 8707],
    ["empty", 8709],
    ["nabla", 8711],
    ["isin", 8712],
    ["notin", 8713],
    ["ni", 8715],
    ["prod", 8719],
    ["sum", 8721],
    ["minus", 8722],
    ["lowast", 8727],
    ["radic", 8730],
    ["prop", 8733],
    ["infin", 8734],
    ["ang", 8736],
    ["and", 8743],
    ["or", 8744],
    ["cap", 8745],
    ["cup", 8746],
    ["int", 8747],
    ["there4", 8756],
    ["sim", 8764],
    ["cong", 8773],
    ["asymp", 8776],
    ["ne", 8800],
    ["equiv", 8801],
    ["le", 8804],
    ["ge", 8805],
    ["sub", 8834],
    ["sup", 8835],
    ["nsub", 8836],
    ["sube", 8838],
    ["supe", 8839],
    ["oplus", 8853],
    ["otimes", 8855],
    ["perp", 8869],
    ["sdot", 8901],
    ["lceil", 8968],
    ["rceil", 8969],
    ["lfloor", 8970],
    ["rfloor", 8971],
    ["lang", 9001],
    ["rang", 9002],
    ["loz", 9674],
    ["spades", 9824],
    ["clubs", 9827],
    ["hearts", 9829],
    ["diams", 9830],
    ["OElig", 338],
    ["oelig", 339],
    ["Scaron", 352],
    ["scaron", 353],
    ["Yuml", 376],
    ["circ", 710],
    ["tilde", 732],
    ["ensp", 8194],
    ["emsp", 8195],
    ["thinsp", 8201],
    ["zwnj", 8204],
    ["zwj", 8205],
    ["lrm", 8206],
    ["rlm", 8207],
    ["ndash", 8211],
    ["mdash", 8212],
    ["lsquo", 8216],
    ["rsquo", 8217],
    ["sbquo", 8218],
    ["ldquo", 8220],
    ["rdquo", 8221],
    ["bdquo", 8222],
    ["dagger", 8224],
    ["Dagger", 8225],
    ["permil", 8240],
    ["lsaquo", 8249],
    ["rsaquo", 8250],
    ["euro", 8364],
  ]
  for (const [name, code] of extra) {
    map.set(name, String.fromCodePoint(code))
  }
  return map
}
