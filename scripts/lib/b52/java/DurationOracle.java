// The oracle for packages/javaduration: Java 17's java.time.Duration and
// java.time.Period. Text is hex UTF-8, '-' when empty.
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.time.Period;

public class DurationOracle {
    static String hx(String s) {
        if (s.isEmpty()) return "-";
        StringBuilder b = new StringBuilder();
        for (byte x : s.getBytes(StandardCharsets.UTF_8)) b.append(String.format("%02x", x & 0xff));
        return b.toString();
    }

    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }

    static String dur(Duration d) {
        return d.getSeconds() + " " + d.getNano() + " " + hx(d.toString());
    }

    static String per(Period p) {
        return p.getYears() + " " + p.getMonths() + " " + p.getDays() + " " + hx(p.toString());
    }

    static String line(String[] f) {
        switch (f[0]) {
            case "dp": return dur(Duration.parse(unhex(f[1])));
            case "dt": return dur(Duration.ofSeconds(Long.parseLong(f[1]), Long.parseLong(f[2])));
            case "da": {
                Duration a = Duration.ofSeconds(Long.parseLong(f[2]), Long.parseLong(f[3]));
                switch (f[1]) {
                    case "plus": return dur(a.plus(Duration.ofSeconds(Long.parseLong(f[4]), Long.parseLong(f[5]))));
                    case "mul": return dur(a.multipliedBy(Long.parseLong(f[4])));
                    case "div": return dur(a.dividedBy(Long.parseLong(f[4])));
                    case "divd": return Long.toString(a.dividedBy(Duration.ofSeconds(Long.parseLong(f[4]), Long.parseLong(f[5]))));
                    case "neg": return dur(a.negated());
                    case "millis": return Long.toString(a.toMillis());
                    default: return Long.toString(a.toNanos());
                }
            }
            case "pp": return per(Period.parse(unhex(f[1])));
            default: {
                Period p = Period.of(Integer.parseInt(f[2]), Integer.parseInt(f[3]), Integer.parseInt(f[4]));
                switch (f[1]) {
                    case "plus": return per(p.plus(Period.of(Integer.parseInt(f[5]), Integer.parseInt(f[6]), Integer.parseInt(f[7]))));
                    case "mul": return per(p.multipliedBy(Integer.parseInt(f[5])));
                    case "norm": return per(p.normalized());
                    default: return per(p);
                }
            }
        }
    }

    public static void main(String[] args) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        StringBuilder out = new StringBuilder();
        String l;
        while ((l = in.readLine()) != null) {
            if (l.isEmpty()) continue;
            String r;
            try {
                r = line(l.split(" "));
            } catch (java.time.format.DateTimeParseException e) {
                r = "Eparse:" + hx(e.getMessage());
            } catch (ArithmeticException e) {
                r = "Earith:" + hx(e.getMessage());
            }
            out.append(r).append('\n');
        }
        System.out.print(out);
    }
}
