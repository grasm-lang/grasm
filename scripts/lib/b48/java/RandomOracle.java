// The oracle for packages/javarandom: java.util.Random, SplittableRandom
// and Collections.shuffle. Each input line is `<kind> <seed> <op>...`; each
// answer line is the results, space-separated. Doubles print as their
// IEEE-754 bits in hex.
import java.io.*;
import java.util.*;

public class RandomOracle {
    static String d(double x) { return Long.toHexString(Double.doubleToRawLongBits(x)); }

    public static void main(String[] a) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in));
        String line;
        StringBuilder all = new StringBuilder();
        while ((line = in.readLine()) != null) {
            if (line.isEmpty()) continue;
            String[] f = line.split(" ");
            long seed = Long.parseLong(f[1]);
            List<String> out = new ArrayList<>();
            if (f[0].equals("R")) {
                Random r = new Random(seed);
                for (int k = 2; k < f.length; k++) {
                    String[] op = f[k].split(":");
                    switch (op[0]) {
                        case "i": out.add(Integer.toString(r.nextInt())); break;
                        case "ib": out.add(Integer.toString(r.nextInt(Integer.parseInt(op[1])))); break;
                        case "ir": out.add(Integer.toString(r.nextInt(Integer.parseInt(op[1]), Integer.parseInt(op[2])))); break;
                        case "l": out.add(Long.toString(r.nextLong())); break;
                        case "lb": out.add(Long.toString(r.nextLong(Long.parseLong(op[1])))); break;
                        case "z": out.add(r.nextBoolean() ? "t" : "f"); break;
                        case "f": out.add(d(r.nextFloat())); break;
                        case "d": out.add(d(r.nextDouble())); break;
                        case "g": out.add(d(r.nextGaussian())); break;
                        case "b": { byte[] b = new byte[Integer.parseInt(op[1])]; r.nextBytes(b); StringBuilder s = new StringBuilder("x");
                            for (byte x : b) s.append(String.format("%02x", x)); out.add(s.toString()); break; }
                        case "s": { List<Integer> xs = new ArrayList<>(); for (int i = 0; i < Integer.parseInt(op[1]); i++) xs.add(i);
                            Collections.shuffle(xs, r); StringBuilder s = new StringBuilder("p"); for (int x : xs) s.append(x).append(','); out.add(s.toString()); break; }
                    }
                }
            } else {
                SplittableRandom r = new SplittableRandom(seed);
                for (int k = 2; k < f.length; k++) {
                    String[] op = f[k].split(":");
                    switch (op[0]) {
                        case "i": out.add(Integer.toString(r.nextInt())); break;
                        case "ib": out.add(Integer.toString(r.nextInt(Integer.parseInt(op[1])))); break;
                        case "l": out.add(Long.toString(r.nextLong())); break;
                        case "z": out.add(r.nextBoolean() ? "t" : "f"); break;
                        case "f": out.add(d(r.nextFloat())); break;
                        case "d": out.add(d(r.nextDouble())); break;
                        case "sp": r = r.split(); out.add("split"); break;
                    }
                }
            }
            all.append(String.join(" ", out)).append('\n');
        }
        System.out.print(all);
    }
}
