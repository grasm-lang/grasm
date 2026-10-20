// The oracle for packages/javauri. Each input line is an operation on hex
// arguments (`~` for null): `p s`, `r base child`, `l base child`,
// `q a b`, `a s` (parseServerAuthority), `c7 scheme userInfo host port path
// query fragment`, `c5 scheme authority path query fragment`, `c3 scheme ssp
// fragment`. Each answer is one line: a URI's every getter, hex, or the
// failure Java reports.
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;

public class UriOracle {
    static String unhex(String h) {
        if (h.equals("~")) return null;
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }
    static String hex(String s) {
        if (s == null) return "~";
        StringBuilder h = new StringBuilder();
        for (byte y : s.getBytes(StandardCharsets.UTF_8)) h.append(String.format("%02x", y));
        return h.length() == 0 ? "-" : h.toString();
    }
    static String desc(URI u) {
        String raw = u.getRawSchemeSpecificPart();
        String[] f = { u.getScheme(), raw, u.getRawAuthority(), u.getRawUserInfo(), u.getHost(), String.valueOf(u.getPort()), u.getRawPath(),
            u.getRawQuery(), u.getRawFragment(), u.getSchemeSpecificPart(), u.getAuthority(), u.getUserInfo(), u.getPath(), u.getQuery(),
            u.getFragment(), u.toString(), u.toASCIIString(), u.normalize().toString(), String.valueOf(u.hashCode()), String.valueOf(u.isOpaque()) };
        StringBuilder out = new StringBuilder();
        for (String s : f) out.append(out.length() == 0 ? "" : ",").append(hex(s));
        return out.toString();
    }
    static String run(String[] f) throws Exception {
        switch (f[0]) {
            case "p": return desc(new URI(unhex(f[1])));
            case "a": return desc(new URI(unhex(f[1])).parseServerAuthority());
            case "r": return desc(new URI(unhex(f[1])).resolve(new URI(unhex(f[2]))));
            case "l": return desc(new URI(unhex(f[1])).relativize(new URI(unhex(f[2]))));
            case "q": {
                URI a = new URI(unhex(f[1])), b = new URI(unhex(f[2]));
                return a.equals(b) + " " + a.compareTo(b) + " " + a.hashCode() + " " + b.hashCode();
            }
            case "c7": return desc(new URI(unhex(f[1]), unhex(f[2]), unhex(f[3]), Integer.parseInt(f[4]), unhex(f[5]), unhex(f[6]), unhex(f[7])));
            case "c5": return desc(new URI(unhex(f[1]), unhex(f[2]), unhex(f[3]), unhex(f[4]), unhex(f[5])));
            default: return desc(new URI(unhex(f[1]), unhex(f[2]), unhex(f[3])));
        }
    }
    public static void main(String[] x) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        PrintStream out = new PrintStream(new FileOutputStream(FileDescriptor.out), false, "UTF-8");
        String line;
        while ((line = in.readLine()) != null) {
            if (line.isEmpty()) continue;
            String r;
            try {
                r = run(line.split(" "));
            } catch (URISyntaxException e) {
                r = "E " + e.getIndex() + " " + hex(e.getReason());
            } catch (NumberFormatException e) {
                r = "N";
            } catch (Exception e) {
                r = "X " + e.getClass().getSimpleName();
            }
            out.println(r);
        }
        out.flush();
    }
}
