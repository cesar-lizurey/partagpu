import { useState, useEffect, useRef } from "react";

interface ResourceSlidersProps {
  cpuLimit: number;
  ramLimitMb: number;
  gpuLimit: number;
  ramTotalMb: number;
  gpuAvailable: boolean;
  onChange: (cpu: number, ram: number, gpu: number) => void;
}

export function ResourceSliders({
  cpuLimit,
  ramLimitMb,
  gpuLimit,
  ramTotalMb,
  gpuAvailable,
  onChange,
}: ResourceSlidersProps) {
  // Local state for instant visual feedback
  const [localCpu, setLocalCpu] = useState(cpuLimit);
  const [localRam, setLocalRam] = useState(ramLimitMb);
  const [localGpu, setLocalGpu] = useState(gpuLimit);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Sync local state when props change (e.g. after backend confirms)
  useEffect(() => setLocalCpu(cpuLimit), [cpuLimit]);
  useEffect(() => setLocalRam(ramLimitMb), [ramLimitMb]);
  useEffect(() => setLocalGpu(gpuLimit), [gpuLimit]);

  const scheduleCommit = (cpu: number, ram: number, gpu: number) => {
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => onChange(cpu, ram, gpu), 300);
  };

  const handleCpu = (val: number) => {
    setLocalCpu(val);
    scheduleCommit(val, localRam, localGpu);
  };

  const handleRam = (val: number) => {
    setLocalRam(val);
    scheduleCommit(localCpu, val, localGpu);
  };

  const handleGpu = (val: number) => {
    setLocalGpu(val);
    scheduleCommit(localCpu, localRam, val);
  };

  return (
    <div className="resource-sliders">
      <h3>Limites de partage</h3>

      <div className="slider-group">
        <label>
          CPU : <strong>{localCpu}%</strong>
        </label>
        <input
          type="range"
          min="0"
          max="100"
          step="5"
          value={localCpu}
          onChange={(e) => handleCpu(Number(e.target.value))}
        />
      </div>

      <div className="slider-group">
        <label>
          RAM :{" "}
          <strong>
            {localRam > 0 ? `${localRam} Mo` : "Illimitée"}
          </strong>
          {ramTotalMb > 0 && (
            <span className="slider-group__hint">
              {" "}
              / {ramTotalMb} Mo total
            </span>
          )}
        </label>
        <input
          type="range"
          min="0"
          max={ramTotalMb}
          step="256"
          value={localRam}
          onChange={(e) => handleRam(Number(e.target.value))}
        />
      </div>

      {gpuAvailable && (
        <div className="slider-group">
          <label>
            GPU : <strong>{localGpu}%</strong>
          </label>
          <input
            type="range"
            min="0"
            max="100"
            step="5"
            value={localGpu}
            onChange={(e) => handleGpu(Number(e.target.value))}
          />
        </div>
      )}
    </div>
  );
}
